//! Scheduled backups: a consistent copy of the store and the settings files, pruned to the
//! newest few, taken inside the running process so the board never stops for it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::api::Shared;
use crate::board::system_ms;

const PREFIX: &str = "board-";

/// Where backups go, how many to keep, and the settings files that travel with the store.
pub struct Backups {
    dir: PathBuf,
    keep: usize,
    extras: Vec<PathBuf>,
}

fn text(path: &Path, e: impl std::fmt::Display) -> String {
    format!("{}: {e}", path.display())
}

impl Backups {
    /// Keep the newest `keep` (at least one) backups in `dir`. `extras` that exist when a
    /// backup runs are copied owner-only: they hold credentials.
    pub fn new(dir: PathBuf, keep: usize, extras: Vec<PathBuf>) -> Self {
        Self {
            dir,
            keep: keep.max(1),
            extras,
        }
    }

    /// Take one backup, prune the old ones, then compact the store. Returns the new backup's
    /// directory. A backup is built under a temporary name and renamed, so an interrupted
    /// run never leaves something that looks complete.
    ///
    /// # Errors
    /// A message naming the file or step that failed; the previous backups are untouched.
    pub fn run(&self, board: &Shared) -> Result<PathBuf, String> {
        let name = format!("{PREFIX}{:013}", system_ms());
        let (tmp, done) = (self.dir.join(format!(".{name}")), self.dir.join(&name));
        std::fs::create_dir_all(&tmp).map_err(|e| text(&tmp, e))?;
        self.fill(board, &tmp).inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&tmp);
        })?;
        std::fs::rename(&tmp, &done).map_err(|e| text(&done, e))?;
        self.prune()?;
        board.compact().map_err(|e| e.to_string())?;
        Ok(done)
    }

    fn fill(&self, board: &Shared, into: &Path) -> Result<(), String> {
        board.backup(into).map_err(|e| e.to_string())?;
        for from in self.extras.iter().filter(|p| p.is_file()) {
            let bytes = std::fs::read(from).map_err(|e| text(from, e))?;
            let to = into.join(from.file_name().ok_or_else(|| text(from, "no file name"))?);
            rusty_atomic_file::write_private(&to, &bytes).map_err(|e| text(&to, e))?;
        }
        Ok(())
    }

    /// Remove all but the newest `keep` backups. The names sort by time.
    fn prune(&self) -> Result<(), String> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map_err(|e| text(&self.dir, e))?
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(PREFIX))
            .map(|e| e.path())
            .collect();
        found.sort();
        let excess = found.len().saturating_sub(self.keep);
        for old in &found[..excess] {
            std::fs::remove_dir_all(old).map_err(|e| text(old, e))?;
        }
        Ok(())
    }

    /// Run [`Backups::run`] every `every` on a background thread, logging each outcome.
    ///
    /// # Errors
    /// If the thread cannot start.
    pub fn start(self, board: Arc<Shared>, every: Duration) -> Result<(), String> {
        std::thread::Builder::new()
            .name("backups".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(every);
                    match self.run(&board) {
                        Ok(done) => tracing::info!(backup = %done.display(), "backup taken"),
                        Err(why) => tracing::error!(%why, "backup failed"),
                    }
                }
            })
            .map(drop)
            .map_err(|e| format!("backup thread: {e}"))
    }
}
