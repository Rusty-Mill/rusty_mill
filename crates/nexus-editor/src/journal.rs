//! Crash journal for open editor sessions (RFC 0009 row 5).
//!
//! Every successful mutation of a file-backed session (`apply_transaction`,
//! `undo`, `redo`, `sync_content`, `stamp_block`) rewrites one small file,
//! `<forge>/.forge/.editor/journal/<key>.json`, holding the session's
//! current canonical markdown together with `base_hash`, the SHA-256 of
//! the bytes that were on disk when the session was opened or last
//! saved. The write is temp + rename, so the file is always either the
//! previous complete record or the new one, never a torn mix.
//!
//! On `open`, a journal whose `base_hash` matches the bytes now on disk
//! is replayed: the session starts from the journal's content instead
//! of the file's, and the snapshot reports `recovered_unsaved_edits`.
//! Any other journal (file changed on disk since, different version,
//! nothing to recover) is discarded — the file on disk wins, per the
//! file-as-truth invariant. `save` and `close` delete the journal.
//!
//! `nexus_forge` journaled per-op deltas against a Brain-owned rope.
//! Nexus block IDs are minted fresh on every parse, so replaying ops
//! against a re-parsed tree is not possible; a full-content snapshot
//! sidesteps that and needs no CRC framing because the rename makes
//! each record atomic. Writes go through the page cache without an
//! fsync: they survive process death (the case the crash test models)
//! but not power loss — see `docs/0.1.2/settings/hardcoded-rust.md`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::handlers::session::content_hash_hex;

/// Forge-relative directory holding one journal file per open note.
pub const JOURNAL_DIR_REL: &str = ".forge/.editor/journal";

const JOURNAL_VERSION: u32 = 1;

/// On-disk record. `content` is the whole canonical markdown of the
/// session at the time of the last mutation.
#[derive(Debug, Serialize, Deserialize)]
struct JournalFile {
    version: u32,
    relpath: String,
    /// SHA-256 hex of the on-disk bytes the session was opened from or
    /// last saved to. Recovery replays only when this still matches
    /// the file, so an external edit invalidates the journal.
    base_hash: String,
    content: String,
    recorded_at_unix: u64,
}

/// Stable 16-hex-char key for a forge-relative path, shared with the
/// BL-072 undo files so the two sidecars for one note line up.
#[must_use]
pub(crate) fn relpath_key(relpath: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let digest = Sha256::digest(relpath.as_bytes());
    let mut hex = String::with_capacity(16);
    for b in digest.iter().take(8) {
        write!(&mut hex, "{b:02x}").expect("write to String");
    }
    hex
}

/// Per-forge journal handle. Cheap to clone via `Arc`; holds no state
/// beyond the forge root.
#[derive(Debug)]
pub struct EditJournal {
    forge_root: PathBuf,
}

impl EditJournal {
    /// Journal rooted at `forge_root`. Nothing is created until the
    /// first [`record`](Self::record).
    #[must_use]
    pub fn new(forge_root: PathBuf) -> Self {
        Self { forge_root }
    }

    /// Absolute path of the journal file for `relpath`.
    #[must_use]
    pub fn path_for(&self, relpath: &str) -> PathBuf {
        self.forge_root
            .join(JOURNAL_DIR_REL)
            .join(format!("{}.json", relpath_key(relpath)))
    }

    /// Atomically replace the journal for `relpath` with `content`.
    ///
    /// # Errors
    ///
    /// Any I/O failure creating the directory, writing the temp file,
    /// or renaming it into place.
    pub fn record(&self, relpath: &str, base_hash: &str, content: &str) -> io::Result<()> {
        let path = self.path_for(relpath);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = JournalFile {
            version: JOURNAL_VERSION,
            relpath: relpath.to_string(),
            base_hash: base_hash.to_string(),
            content: content.to_string(),
            recorded_at_unix: now_unix_secs(),
        };
        let bytes = serde_json::to_vec(&file).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)
    }

    /// Content to restore for `relpath` given the bytes now on disk, or
    /// `None` when there is nothing valid to recover. A journal that is
    /// unreadable, from another version, for another path, stale
    /// (base hash no longer matches the file) or identical to the file
    /// is deleted on the way out.
    #[must_use]
    pub fn recover(&self, relpath: &str, disk_bytes: &[u8]) -> Option<String> {
        let path = self.path_for(relpath);
        let bytes = fs::read(&path).ok()?;
        let parsed: Option<JournalFile> = serde_json::from_slice(&bytes).ok();
        let recovered = parsed.and_then(|j| {
            let matches = j.version == JOURNAL_VERSION
                && j.relpath == relpath
                && j.base_hash == content_hash_hex(disk_bytes)
                && j.content.as_bytes() != disk_bytes;
            matches.then_some(j.content)
        });
        if recovered.is_none() {
            let _ = fs::remove_file(&path);
        }
        recovered
    }

    /// Delete the journal for `relpath`, if any. Missing is not an error.
    pub fn clear(&self, relpath: &str) {
        let path = self.path_for(relpath);
        if let Err(err) = fs::remove_file(&path) {
            if err.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    relpath,
                    path = %path.display(),
                    %err,
                    "edit journal: failed to remove journal file"
                );
            }
        }
    }

    /// Forge root this journal writes under.
    #[must_use]
    pub fn forge_root(&self) -> &Path {
        &self.forge_root
    }
}

fn now_unix_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> (tempfile::TempDir, EditJournal) {
        let dir = tempfile::tempdir().expect("tempdir");
        let j = EditJournal::new(dir.path().to_path_buf());
        (dir, j)
    }

    #[test]
    fn relpath_key_is_stable_and_short() {
        assert_eq!(relpath_key("a.md"), relpath_key("a.md"));
        assert_ne!(relpath_key("a.md"), relpath_key("b.md"));
        assert_eq!(relpath_key("a.md").len(), 16);
    }

    #[test]
    fn record_then_recover_against_matching_disk_bytes() {
        let (_dir, j) = journal();
        let disk = b"Hello\n";
        j.record("n.md", &content_hash_hex(disk), "Hello world\n")
            .unwrap();
        assert_eq!(j.recover("n.md", disk).as_deref(), Some("Hello world\n"));
        // Recovery keeps the journal until save/close clears it.
        assert!(j.path_for("n.md").exists());
        j.clear("n.md");
        assert!(!j.path_for("n.md").exists());
        assert_eq!(j.recover("n.md", disk), None);
    }

    #[test]
    fn stale_or_pointless_journals_are_discarded() {
        let (_dir, j) = journal();
        let disk = b"Hello\n";
        // File changed on disk since the journal was written.
        j.record("n.md", &content_hash_hex(b"old\n"), "old edit\n")
            .unwrap();
        assert_eq!(j.recover("n.md", disk), None);
        assert!(!j.path_for("n.md").exists(), "stale journal removed");

        // Nothing to recover: content already equals the file.
        j.record("n.md", &content_hash_hex(disk), "Hello\n")
            .unwrap();
        assert_eq!(j.recover("n.md", disk), None);
        assert!(!j.path_for("n.md").exists());

        // Garbage on disk.
        fs::create_dir_all(j.path_for("n.md").parent().unwrap()).unwrap();
        fs::write(j.path_for("n.md"), b"{not json").unwrap();
        assert_eq!(j.recover("n.md", disk), None);
        assert!(!j.path_for("n.md").exists());
    }

    #[test]
    fn record_replaces_atomically_leaving_no_temp_file() {
        let (_dir, j) = journal();
        let base = content_hash_hex(b"x\n");
        j.record("n.md", &base, "one\n").unwrap();
        j.record("n.md", &base, "two\n").unwrap();
        assert_eq!(j.recover("n.md", b"x\n").as_deref(), Some("two\n"));
        assert!(!j.path_for("n.md").with_extension("json.tmp").exists());
    }

    #[test]
    fn clear_on_missing_is_silent() {
        let (_dir, j) = journal();
        j.clear("never.md");
    }
}
