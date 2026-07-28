//! Git's index (staging area): the real binary index-v2 format (a `DIRC`
//! header, fixed-width entries, a trailing SHA-1 checksum), not an
//! in-memory placeholder — real `git status`/`git ls-files --stage` can
//! read an index this module writes.
//!
//! Known simplification: `dev`/`ino`/`uid`/`gid` are always written as `0`.
//! Real git uses them only as a fast-path "has this file possibly changed"
//! heuristic (falling back to a full content hash comparison either way),
//! not for correctness, so zeroing them is safe but means this
//! implementation always takes the slow (hash-compare) path rather than
//! git's usual stat-cache fast path.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::sha1::{sha1, Sha1, SHA1_DIGEST_LEN};

const DIRC_MAGIC: &[u8; 4] = b"DIRC";
const INDEX_VERSION: u32 = 2;
/// Fixed-width portion of one entry: 10 `u32`s (40 bytes) + a 20-byte SHA-1
/// + a 2-byte flags field.
const ENTRY_FIXED_LEN: usize = 4 * 10 + SHA1_DIGEST_LEN + 2;

/// Regular, non-executable file mode (git's own encoding: object-type
/// nibble `1000` plus Unix permission bits).
pub const MODE_REGULAR: u32 = 0o100644;
/// Regular, executable file mode.
pub const MODE_EXECUTABLE: u32 = 0o100755;

/// One staged file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Git's mode encoding ([`MODE_REGULAR`] or [`MODE_EXECUTABLE`]).
    pub mode: u32,
    /// File size in bytes, at staging time.
    pub size: u32,
    /// Raw 20-byte SHA-1 of the blob object this entry's content hashes to.
    pub hash: [u8; SHA1_DIGEST_LEN],
    /// Path relative to the repository root, using `/` separators.
    pub path: String,
}

/// Errors reading an index file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    /// Missing/bad `DIRC` magic, or an unsupported version.
    BadHeader,
    /// An entry's fixed fields or name ran past the end of the file.
    Truncated,
    /// The trailing SHA-1 checksum didn't match the file's actual content.
    ChecksumMismatch,
}

impl core::fmt::Display for IndexError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IndexError::BadHeader => write!(f, "not a version-2 git index (bad DIRC header)"),
            IndexError::Truncated => write!(f, "index file truncated"),
            IndexError::ChecksumMismatch => write!(f, "index checksum mismatch"),
        }
    }
}

impl std::error::Error for IndexError {}

/// The staging area: a sorted-by-path set of [`IndexEntry`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    entries: Vec<IndexEntry>,
}

impl Index {
    /// An empty index (a freshly-initialized repository's staging area).
    pub fn new() -> Self {
        Index { entries: Vec::new() }
    }

    /// The staged entries, sorted by path (git's own required order).
    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }

    /// Stages `entry`, replacing any existing entry for the same path and
    /// keeping entries sorted by path.
    pub fn upsert(&mut self, entry: IndexEntry) {
        match self.entries.binary_search_by(|e| e.path.cmp(&entry.path)) {
            Ok(i) => self.entries[i] = entry,
            Err(i) => self.entries.insert(i, entry),
        }
    }

    /// Removes the entry for `path`, if staged. Returns whether one was
    /// removed.
    pub fn remove(&mut self, path: &str) -> bool {
        if let Ok(i) = self.entries.binary_search_by(|e| e.path.as_str().cmp(path)) {
            self.entries.remove(i);
            true
        } else {
            false
        }
    }

    /// Looks up the staged entry for `path`, if any.
    pub fn get(&self, path: &str) -> Option<&IndexEntry> {
        self.entries
            .binary_search_by(|e| e.path.as_str().cmp(path))
            .ok()
            .map(|i| &self.entries[i])
    }

    /// Serializes this index to real git index-v2 binary format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(DIRC_MAGIC);
        out.extend_from_slice(&INDEX_VERSION.to_be_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_be_bytes());

        for entry in &self.entries {
            // ctime/mtime: this crate doesn't track real stat times (see
            // module doc on the dev/ino/uid/gid simplification) -- zeroed,
            // which just means this entry never satisfies git's stat-cache
            // fast path and always falls back to a real content comparison.
            for _ in 0..4 {
                out.extend_from_slice(&0u32.to_be_bytes()); // ctime s/ns, mtime s/ns
            }
            out.extend_from_slice(&0u32.to_be_bytes()); // dev
            out.extend_from_slice(&0u32.to_be_bytes()); // ino
            out.extend_from_slice(&entry.mode.to_be_bytes());
            out.extend_from_slice(&0u32.to_be_bytes()); // uid
            out.extend_from_slice(&0u32.to_be_bytes()); // gid
            out.extend_from_slice(&entry.size.to_be_bytes());
            out.extend_from_slice(&entry.hash);

            let name_bytes = entry.path.as_bytes();
            let name_len = name_bytes.len().min(0xFFF) as u16;
            out.extend_from_slice(&name_len.to_be_bytes());
            out.extend_from_slice(name_bytes);

            // Pad with NULs (at least one, the name terminator) so the
            // whole entry is a multiple of 8 bytes.
            let unpadded = ENTRY_FIXED_LEN + name_bytes.len();
            let mut pad = 8 - (unpadded % 8);
            if pad == 0 {
                pad = 8;
            }
            out.extend(std::iter::repeat_n(0u8, pad));
        }

        let checksum = sha1(&out);
        out.extend_from_slice(&checksum);
        out
    }

    /// Parses a real git index-v2 file's bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, IndexError> {
        if data.len() < 12 + SHA1_DIGEST_LEN {
            return Err(IndexError::Truncated);
        }
        if &data[0..4] != DIRC_MAGIC {
            return Err(IndexError::BadHeader);
        }
        let version = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        if version != 2 {
            // Versions 3/4 add extensions/path-compression this parser
            // doesn't implement; only v2 is supported.
            return Err(IndexError::BadHeader);
        }
        let count = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;

        let content_len = data.len() - SHA1_DIGEST_LEN;
        let expected_checksum = &data[content_len..];
        let mut hasher = Sha1::new();
        hasher.update(&data[..content_len]);
        if hasher.finish() != expected_checksum {
            return Err(IndexError::ChecksumMismatch);
        }

        let mut entries = Vec::with_capacity(count);
        let mut cursor = 12;
        for _ in 0..count {
            if cursor + ENTRY_FIXED_LEN > content_len {
                return Err(IndexError::Truncated);
            }
            let field = |off: usize| -> u32 {
                u32::from_be_bytes([
                    data[cursor + off],
                    data[cursor + off + 1],
                    data[cursor + off + 2],
                    data[cursor + off + 3],
                ])
            };
            // Layout: ctime_sec(0) ctime_nsec(4) mtime_sec(8) mtime_nsec(12)
            // dev(16) ino(20) mode(24) uid(28) gid(32) size(36).
            let mode = field(24);
            let size = field(36);
            let mut hash = [0u8; SHA1_DIGEST_LEN];
            hash.copy_from_slice(&data[cursor + 40..cursor + 40 + SHA1_DIGEST_LEN]);
            let flags_off = cursor + 40 + SHA1_DIGEST_LEN;
            let flags = u16::from_be_bytes([data[flags_off], data[flags_off + 1]]);
            let name_len = (flags & 0x0FFF) as usize;

            let name_start = flags_off + 2;
            if name_start + name_len > content_len {
                return Err(IndexError::Truncated);
            }
            let path = std::str::from_utf8(&data[name_start..name_start + name_len])
                .map_err(|_| IndexError::Truncated)?
                .to_string();

            let unpadded = ENTRY_FIXED_LEN + name_len;
            let mut pad = 8 - (unpadded % 8);
            if pad == 0 {
                pad = 8;
            }
            cursor += unpadded + pad;

            entries.push(IndexEntry { mode, size, hash, path });
        }

        Ok(Index { entries })
    }

    /// Reads the index file at `git_dir/index`, or an empty index if none
    /// exists yet (a freshly-initialized repository).
    pub fn read(git_dir: &Path) -> Result<Self, IndexError> {
        let path = git_dir.join("index");
        let mut file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return Ok(Index::new()),
        };
        let mut data = Vec::new();
        file.read_to_end(&mut data).map_err(|_| IndexError::Truncated)?;
        Self::from_bytes(&data)
    }

    /// Writes this index to `git_dir/index`.
    pub fn write(&self, git_dir: &Path) -> std::io::Result<()> {
        fs::write(git_dir.join("index"), self.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> IndexEntry {
        IndexEntry {
            mode: MODE_REGULAR,
            size: 11,
            hash: sha1(b"hello world"),
            path: path.to_string(),
        }
    }

    #[test]
    fn empty_index_round_trips() {
        let index = Index::new();
        let bytes = index.to_bytes();
        let back = Index::from_bytes(&bytes).unwrap();
        assert_eq!(back, index);
    }

    #[test]
    fn single_entry_round_trips() {
        let mut index = Index::new();
        index.upsert(entry("README.md"));
        let bytes = index.to_bytes();
        let back = Index::from_bytes(&bytes).unwrap();
        assert_eq!(back, index);
        assert_eq!(back.entries()[0].path, "README.md");
    }

    #[test]
    fn entries_stay_sorted_and_upsert_replaces() {
        let mut index = Index::new();
        index.upsert(entry("b.txt"));
        index.upsert(entry("a.txt"));
        index.upsert(entry("c.txt"));
        let paths: Vec<&str> = index.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a.txt", "b.txt", "c.txt"]);

        let mut replaced = entry("b.txt");
        replaced.size = 999;
        index.upsert(replaced);
        assert_eq!(index.get("b.txt").unwrap().size, 999);
        assert_eq!(index.entries().len(), 3);
    }

    #[test]
    fn remove_deletes_the_right_entry() {
        let mut index = Index::new();
        index.upsert(entry("a.txt"));
        index.upsert(entry("b.txt"));
        assert!(index.remove("a.txt"));
        assert!(!index.remove("a.txt"));
        assert_eq!(index.entries().len(), 1);
        assert_eq!(index.entries()[0].path, "b.txt");
    }

    #[test]
    fn corrupted_checksum_is_rejected() {
        let mut index = Index::new();
        index.upsert(entry("a.txt"));
        let mut bytes = index.to_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert_eq!(Index::from_bytes(&bytes), Err(IndexError::ChecksumMismatch));
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut bytes = Index::new().to_bytes();
        bytes[0] = b'X';
        // Recompute checksum so BadHeader (not ChecksumMismatch) is what's hit.
        let content_len = bytes.len() - SHA1_DIGEST_LEN;
        let checksum = sha1(&bytes[..content_len]);
        bytes[content_len..].copy_from_slice(&checksum);
        assert_eq!(Index::from_bytes(&bytes), Err(IndexError::BadHeader));
    }

    #[test]
    fn read_returns_empty_index_when_no_file_exists() {
        let dir = std::env::temp_dir().join(format!("rusty_git_index_test_missing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let index = Index::read(&dir).unwrap();
        assert_eq!(index, Index::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_then_read_round_trips_on_disk() {
        let dir = std::env::temp_dir().join(format!("rusty_git_index_test_disk_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut index = Index::new();
        index.upsert(entry("a.txt"));
        index.upsert(entry("dir/b.txt"));
        index.write(&dir).unwrap();

        let back = Index::read(&dir).unwrap();
        assert_eq!(back, index);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
