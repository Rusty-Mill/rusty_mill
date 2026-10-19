//! Scheduled backups: a consistent copy of the store and of the settings, pruned to the newest few,
//! taken inside the running process so the board never stops for it.
//!
//! A backup is `board-<unix ms>/` holding `store/` (the store's files, under their own names) and
//! `settings/` (`identities.json`, `projects.json`, `roles.json`, `github.json`, whatever they are
//! called or wherever they live on the host). To restore, put `store/*` where `--store` points and
//! each settings file where its flag (or `--settings-dir`) points.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::api::Shared;
use crate::board::system_ms;
use crate::drain::InFlight;
use crate::mmdb::MmdbStore;
use crate::projects::Projects;
use crate::settings::Settings;
use crate::store::Store;

const PREFIX: &str = "board-";
/// A backup under construction: never mistaken for a finished one, and swept up if a run died.
const TEMP_PREFIX: &str = ".board-";

/// Where backups go and how many to keep.
pub struct Backups {
    dir: PathBuf,
    keep: usize,
}

fn text(path: &Path, e: impl std::fmt::Display) -> String {
    format!("{}: {e}", path.display())
}

/// Make the entries of `dir` durable (file contents were synced as they were written). On Unix
/// this is an fsync of the directory. Elsewhere there is no way to flush a directory through
/// `std`; NTFS and APFS journal renames, so the guarantee there is the filesystem's own.
fn sync_dir(dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)
            .and_then(|d| d.sync_all())
            .map_err(|e| text(dir, e))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// Open the copied store as a restore would, in a scratch directory (opening folds the append log,
/// and the copy itself must stay as it was written), and check that every card belongs to a
/// project of the copied registry. A backup that does not open, or that holds a card whose
/// project the copied settings lack, is no backup.
fn verify(store: &Path, settings: &Path, primary: &OsStr, scratch: &Path) -> Result<(), String> {
    std::fs::create_dir_all(scratch).map_err(|e| text(scratch, e))?;
    let result = check_restored(store, settings, primary, scratch);
    let _ = std::fs::remove_dir_all(scratch);
    result
}

fn check_restored(
    store: &Path,
    settings: &Path,
    primary: &OsStr,
    scratch: &Path,
) -> Result<(), String> {
    for entry in std::fs::read_dir(store).map_err(|e| text(store, e))? {
        let from = entry.map_err(|e| text(store, e))?.path();
        let name = from
            .file_name()
            .ok_or_else(|| text(&from, "no file name"))?;
        std::fs::copy(&from, scratch.join(name)).map_err(|e| text(&from, e))?;
    }
    let main = scratch.join(primary);
    let registry = settings.join("projects.json");
    let projects = if registry.exists() {
        Projects::load(&registry)?
    } else {
        Projects::default()
    };
    let restored = MmdbStore::open(&main).map_err(|e| format!("the copy does not open: {e}"))?;
    let cards = restored.list().map_err(|e| e.to_string())?;
    match cards.iter().find(|c| !projects.contains(&c.project)) {
        Some(card) => Err(format!(
            "the copy holds card {} in project {:?}, which the copied settings lack",
            card.id, card.project
        )),
        None => Ok(()),
    }
}

impl Backups {
    /// Keep the newest `keep` (at least one) backups in `dir`.
    pub fn new(dir: PathBuf, keep: usize) -> Self {
        Self {
            dir,
            keep: keep.max(1),
        }
    }

    /// Take one backup, publish it durably, then prune the old ones and compact the store. Returns
    /// the new backup's directory. It is built under a temporary name and renamed, so an
    /// interrupted run never leaves something that looks complete, and older backups are retired
    /// only once the new one's entries are on disk.
    ///
    /// Refused while the store reports an append of unknown outcome: the files then hold an event
    /// that only a reopen recovers, so nothing may be rotated out, rewritten or compacted first.
    ///
    /// # Errors
    /// A message naming the file or step that failed; the previous backups are untouched.
    pub fn run(&self, board: &Shared, settings: &Settings) -> Result<PathBuf, String> {
        self.run_with(board, settings, &mut || {})
    }

    /// [`Backups::run`], calling `after_store` once the store is copied, with the settings lock
    /// still held. For tests that need a change to arrive between the two copies.
    ///
    /// # Errors
    /// See [`Backups::run`].
    #[doc(hidden)]
    pub fn run_with(
        &self,
        board: &Shared,
        settings: &Settings,
        after_store: &mut dyn FnMut(),
    ) -> Result<PathBuf, String> {
        board
            .health()
            .map_err(|e| format!("not taking a backup: {e}"))?;
        std::fs::create_dir_all(&self.dir).map_err(|e| text(&self.dir, e))?;
        self.sweep_unfinished()?;

        let stamp = format!("{:013}", system_ms());
        let (tmp, done) = (
            self.dir.join(format!("{TEMP_PREFIX}{stamp}")),
            self.dir.join(format!("{PREFIX}{stamp}")),
        );
        self.build(board, settings, &tmp, after_store)
            .inspect_err(|_| {
                let _ = std::fs::remove_dir_all(&tmp);
            })
            .and_then(|()| std::fs::rename(&tmp, &done).map_err(|e| text(&done, e)))?;
        sync_dir(&self.dir)?;
        self.prune()?;
        board
            .compact()
            .map_err(|e| format!("{} was taken, but compacting failed: {e}", done.display()))?;
        Ok(done)
    }

    /// Fill `tmp`, check it, and make it durable. The settings lock is held across the copy of
    /// the store and of the settings, in that order (settings lock, then the store's commit lock,
    /// as every settings change takes them): no project can be removed, or an agent's role
    /// changed, between the cards being copied and the registry they belong to.
    fn build(
        &self,
        board: &Shared,
        settings: &Settings,
        tmp: &Path,
        after_store: &mut dyn FnMut(),
    ) -> Result<(), String> {
        let (store, set) = (tmp.join("store"), tmp.join("settings"));
        for dir in [&store, &set] {
            std::fs::create_dir_all(dir).map_err(|e| text(dir, e))?;
        }
        let mut primary = None;
        settings.snapshot_with(&set, &mut || {
            primary = Some(board.backup(&store).map_err(|e| e.to_string())?);
            after_store();
            Ok(())
        })?;
        let primary = primary.ok_or("the store did not name its main file")?;
        verify(&store, &set, &primary, &tmp.join("verify"))?;
        for dir in [&store, &set, tmp] {
            sync_dir(dir)?;
        }
        Ok(())
    }

    /// Finished backups, oldest first.
    fn finished(&self) -> Result<Vec<PathBuf>, String> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map_err(|e| text(&self.dir, e))?
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(PREFIX))
            .map(|e| e.path())
            .collect();
        found.sort();
        Ok(found)
    }

    /// Remove temporary directories left by a run that died before it finished.
    fn sweep_unfinished(&self) -> Result<(), String> {
        for entry in std::fs::read_dir(&self.dir)
            .map_err(|e| text(&self.dir, e))?
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(TEMP_PREFIX))
        {
            std::fs::remove_dir_all(entry.path()).map_err(|e| text(&entry.path(), e))?;
        }
        Ok(())
    }

    /// Remove all but the newest `keep` backups. The names sort by time.
    fn prune(&self) -> Result<(), String> {
        let found = self.finished()?;
        let excess = found.len().saturating_sub(self.keep);
        for old in &found[..excess] {
            std::fs::remove_dir_all(old).map_err(|e| text(old, e))?;
        }
        sync_dir(&self.dir)
    }

    /// Run [`Backups::run`] every `every` on a background thread, logging each outcome. A run is
    /// counted in `flight`, so a stop waits for it, and none starts once the stop has begun.
    ///
    /// # Errors
    /// If the thread cannot start.
    pub fn start(
        self,
        board: Arc<Shared>,
        settings: Arc<Settings>,
        flight: Arc<InFlight>,
        every: Duration,
    ) -> Result<(), String> {
        std::thread::Builder::new()
            .name("backups".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(every);
                    let Some(_running) = flight.enter() else {
                        return;
                    };
                    match self.run(&board, &settings) {
                        Ok(done) => tracing::info!(backup = %done.display(), "backup taken"),
                        Err(why) => tracing::error!(%why, "backup failed"),
                    }
                }
            })
            .map(drop)
            .map_err(|e| format!("backup thread: {e}"))
    }
}
