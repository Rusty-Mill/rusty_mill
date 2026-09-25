//! The header layout, fingerprint hash and atomic install every companion
//! blob shares: `rusty_multimodal_db`'s Dog `RecordBlob` and this engine's
//! own [`crate::generic`] record and edge blobs. Parameterized by magic and
//! version, so each blob kind keeps its own bytes while sharing one
//! implementation. See `rusty_multimodal_db`'s `durability::record_blob`
//! module docs for the format's rationale (ADR-0016, STORAGE-014).

use super::sync_parent_dir;
use super::DurabilityError;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Every blob magic is 8 bytes.
const MAGIC_LEN: usize = 8;

/// Byte offset of the little-endian `u32` blob version in the header.
pub const VERSION_OFFSET: usize = MAGIC_LEN;

/// Byte offset of the little-endian `u64` fingerprint in the header.
pub const FINGERPRINT_OFFSET: usize = VERSION_OFFSET + 4;

/// Magic, then the blob version as a little-endian `u32`, then the
/// fingerprint as a little-endian `u64`: 20 bytes. The one header layout
/// every companion blob in this crate uses (see module docs, "Shared
/// with the generic store's blob").
pub const HEADER_LEN: usize = FINGERPRINT_OFFSET + 8;

/// FNV-1a 64 — see module docs for why this hash, and why inline. Also
/// an [`io::Write`](std::io::Write) sink, so a `serde` encoder can stream
/// straight into it (the generic blob fingerprints its records that way).
pub struct Fnv1a64(u64);

impl Default for Fnv1a64 {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv1a64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Write for Fnv1a64 {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Validate a fixed [`HEADER_LEN`]-byte header against the `magic` and
/// `expected_version` of the blob kind the caller is reading, and return
/// the fingerprint it records. Shared by `rusty_multimodal_db`'s Dog
/// `RecordBlob::read` (which then decodes the body) and
/// `RecordBlob::is_current_at` (which needs
/// nothing else), and by the generic blob under its own magic. `bytes`
/// may be the whole file or just its first [`HEADER_LEN`] bytes. Errors
/// are the `cause` half of a [`DurabilityError::RecordBlobUnreadable`].
pub fn parse_header(bytes: &[u8], magic: &[u8; 8], expected_version: u32) -> Result<u64, String> {
    if bytes.len() < HEADER_LEN || bytes[0..magic.len()] != magic[..] {
        return Err(
            "magic number mismatch or file too short for a header — not a record blob".to_owned(),
        );
    }
    // Bounds already checked above (`bytes.len() >= HEADER_LEN`), so the
    // fixed-width slices below can't panic — same pattern
    // `read_wal_entries` uses for its length prefix.
    let mut version = [0u8; 4];
    version.copy_from_slice(&bytes[VERSION_OFFSET..FINGERPRINT_OFFSET]);
    let found = u32::from_le_bytes(version);
    if found != expected_version {
        return Err(format!(
            "blob version mismatch: file has {found}, this build expects {expected_version}"
        ));
    }
    let mut fingerprint = [0u8; 8];
    fingerprint.copy_from_slice(&bytes[FINGERPRINT_OFFSET..HEADER_LEN]);
    Ok(u64::from_le_bytes(fingerprint))
}

/// Assemble a complete on-disk image: the [`HEADER_LEN`]-byte header
/// (`magic`, `version`, `fingerprint`, little-endian) followed by `body`.
/// The inverse of [`parse_header`] for the header half; shared for the
/// same reason.
pub fn encode_image(magic: &[u8; 8], version: u32, fingerprint: u64, body: &[u8]) -> Vec<u8> {
    let mut image = Vec::with_capacity(HEADER_LEN + body.len());
    image.extend_from_slice(magic);
    image.extend_from_slice(&version.to_le_bytes());
    image.extend_from_slice(&fingerprint.to_le_bytes());
    image.extend_from_slice(body);
    image
}

/// Suffix appended to the ages file's path to derive the companion
/// blob's — see [`companion_path`].
const COMPANION_SUFFIX: &str = ".records";

/// Suffix appended to the *companion* path for the temp file
/// the Dog `RecordBlob::write` stages into before renaming — mirrors
/// `MmapAgeStore::write_via_rename`'s own `.rewrite-tmp` convention.
pub const TEMP_SUFFIX: &str = ".rewrite-tmp";

/// Where a `ProductionStore` whose ages file lives at `ages_path` keeps
/// its companion record blob: `ages_path` with `COMPANION_SUFFIX`
/// appended (`ages.mmap` → `ages.mmap.records`). A fixed, documented
/// derivation rather than a second caller-supplied path, so the two
/// files are portable as a unit — copy both, reopen with one path
/// (`STORAGE-014-FR-001`). Appending (rather than replacing the
/// extension) means the convention never collides with or depends on
/// whatever extension the caller chose for the ages file. The generic
/// store uses the same derivation for its own companion
/// (`STORAGE-015-FR-001`).
pub fn companion_path(ages_path: &Path) -> PathBuf {
    let mut companion = ages_path.as_os_str().to_owned();
    companion.push(COMPANION_SUFFIX);
    PathBuf::from(companion)
}

/// A companion blob already serialized to its on-disk image.
pub struct EncodedRecordBlob {
    pub image: Vec<u8>,
}

impl EncodedRecordBlob {
    /// Atomically install this image at `path` (already the companion
    /// path — callers derive it via [`companion_path`]), via
    /// write-to-temp-then-rename. See module docs for the crash-safety
    /// argument.
    ///
    /// # Errors
    ///
    /// Returns [`DurabilityError::Io`] if the temp file can't be created,
    /// written, synced, or renamed into place.
    pub fn write(&self, path: &Path) -> Result<(), DurabilityError> {
        let mut temp_path = path.as_os_str().to_owned();
        temp_path.push(TEMP_SUFFIX);
        let temp_path = PathBuf::from(temp_path);

        {
            let mut temp_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temp_path)?;
            temp_file.write_all(&self.image)?;
            temp_file.sync_all()?;
        }
        std::fs::rename(&temp_path, path)?;
        sync_parent_dir(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_path_appends_a_fixed_suffix() {
        assert_eq!(
            companion_path(Path::new("/x/ages.mmap")),
            PathBuf::from("/x/ages.mmap.records")
        );
        // No dependence on the ages file having any particular extension.
        assert_eq!(
            companion_path(Path::new("/x/ages")),
            PathBuf::from("/x/ages.records")
        );
    }
}
