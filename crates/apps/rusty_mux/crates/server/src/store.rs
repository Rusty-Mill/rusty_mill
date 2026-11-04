//! Sessions that survive a server restart (C45): the store directory holds
//! a [`manifest`] and one RTCP v6 screen per block (`<block>.rtcp`).
//!
//! Each file is written whole to a `.tmp` beside it and renamed over the
//! old one; the manifest goes last, so a run that dies midway leaves the
//! previous manifest with screens at least as new. A store that cannot be
//! read is moved aside and the server starts empty: a bad store never
//! stops a start.

pub(crate) mod manifest;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusty_mux_core::BlockId;
use rusty_term_core::{AnsiParser, Grid};

pub(crate) use manifest::{Manifest, Started};

const MANIFEST: &str = "manifest";
const SCREEN: &str = "rtcp";

/// A store directory, made owner-only. Clones share it.
#[derive(Debug, Clone)]
pub(crate) struct Store {
    dir: PathBuf,
    /// The newest save written, so saves land one at a time and an older
    /// one finishing late never overwrites a newer one.
    written: Arc<Mutex<u64>>,
}

/// What a store held: the manifest and each block's screen it could read.
pub(crate) struct Loaded {
    pub(crate) manifest: Manifest,
    pub(crate) screens: Vec<(BlockId, Grid, AnsiParser)>,
}

impl Store {
    /// Opens `dir`, making it (and its parents) if missing.
    pub(crate) fn open(dir: PathBuf) -> io::Result<Self> {
        make_private_dir(&dir)?;
        Ok(Self {
            dir,
            written: Arc::default(),
        })
    }

    fn screen(&self, block: BlockId) -> PathBuf {
        self.dir.join(format!("{block}.{SCREEN}"))
    }

    /// The saved state, `Ok(None)` when there is none. A manifest that
    /// cannot be read is an error; a screen that cannot be read is skipped
    /// with a warning, so its block restarts blank.
    pub(crate) fn load(&self) -> Result<Option<Loaded>, String> {
        let text = match fs::read_to_string(self.dir.join(MANIFEST)) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("reading the manifest: {e}")),
        };
        let manifest = manifest::decode(&text).map_err(|e| e.to_string())?;
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

    /// Writes the screens given, then the manifest, then removes screens
    /// of blocks the manifest no longer holds; skipped, and `Ok`, when a
    /// save numbered `generation` or later has been written already.
    pub(crate) fn save(
        &self,
        generation: u64,
        manifest: &Manifest,
        screens: &[(BlockId, Vec<u8>)],
    ) -> io::Result<()> {
        let mut written = self
            .written
            .lock()
            .map_err(|_| io::Error::other("an earlier save panicked"))?;
        if generation <= *written {
            return Ok(());
        }
        for (block, data) in screens {
            replace(&self.screen(*block), data)?;
        }
        replace(
            &self.dir.join(MANIFEST),
            manifest::encode(manifest).as_bytes(),
        )?;
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if is_stale(&path, manifest) {
                fs::remove_file(&path)?;
            }
        }
        *written = generation;
        Ok(())
    }

    /// Moves an unreadable store to `<dir>.bad-<seconds>` and starts a
    /// new empty one in its place.
    pub(crate) fn set_aside(&self) -> io::Result<PathBuf> {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut aside = self.dir.clone().into_os_string();
        aside.push(format!(".bad-{secs}"));
        let aside = PathBuf::from(aside);
        fs::rename(&self.dir, &aside)?;
        make_private_dir(&self.dir)?;
        Ok(aside)
    }
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
