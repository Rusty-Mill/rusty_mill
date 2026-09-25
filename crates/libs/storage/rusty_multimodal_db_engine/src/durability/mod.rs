//! The error type and on-disk helpers the generic store shares with
//! `rusty_multimodal_db`'s own (Dog) durability variants, extracted with
//! the engine (ADR-0124 in `rusty_multimodal_db`). The Dog variants stay in
//! that crate; they use these through `rusty_multimodal_db::durability`,
//! which re-exports them, so its paths did not change.

pub mod record_blob;

use thiserror::Error;

/// Every fallible outcome across every durability variant. One type,
/// rather than a bespoke error enum per variant — each variant's failure
/// modes (I/O, serialization, an unknown UUID) are the same kinds of
/// thing, just triggered by different code paths.
#[derive(Debug, Error)]
pub enum DurabilityError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("(de)serialization error: {0}")]
    Serde(#[from] bincode::Error),
    /// A consumer store's own error, boxed so this crate needs no knowledge
    /// of it. `rusty_multimodal_db` puts its Dog `StoreError` here (it
    /// implements `From<StoreError>` for this type, and downcasts back).
    #[error("store error: {0}")]
    Store(Box<dyn std::error::Error + Send + Sync + 'static>),
    /// Wraps a Tier 2 variant's own engine error (e.g. `redb`) as a
    /// string — those crates' error types don't all implement a common
    /// trait this crate can `#[from]` directly, and introducing a
    /// per-engine error variant for just one Tier 2 backend isn't worth
    /// the added enum surface.
    #[error("embedded engine error: {0}")]
    Engine(String),
    /// Added for [`crate::generic::mmap_store::GenericMmapStore`]'s
    /// versioned header (`open` checks it before touching any record
    /// data) — the file's first bytes don't match the expected magic
    /// number at all, or the file is too short to even contain a header.
    /// Distinct from [`Self::SchemaVersionMismatch`]: this means "not a
    /// file this store wrote," not "an older version of one." Shared by
    /// every mmap-backed variant that checks a magic number — each uses
    /// its own distinct magic bytes (see each store's own module docs),
    /// so pointing one store's `open` at another's file still fails here
    /// rather than being silently misread. As of
    /// `rusty_multimodal_db`'s `MmapAgeStore`'s own
    /// record-identity-keying port, that's both mmap-backed variants, not
    /// just `GenericMmapStore` — no other (non-mmap) durability variant
    /// writes or checks a magic number at all.
    #[error(
        "not a valid mmap durability file: magic number mismatch or file too short for a header"
    )]
    InvalidMagic,
    /// The file's magic number matches (it *is* a file this store wrote)
    /// but its recorded schema version doesn't match what this build
    /// expects — an older (or, in principle, newer) on-disk layout.
    /// Detection only: no attempt is made to read or migrate the record
    /// data once this fires — see each mmap-backed store's own module
    /// docs for why. Shared by every mmap-backed variant with a versioned
    /// header (`GenericMmapStore` and, as of its own record-identity-keying
    /// port, `MmapAgeStore`) — no other durability variant writes a schema
    /// version.
    #[error("mmap durability file schema version mismatch: file has {found}, this build expects {expected}")]
    SchemaVersionMismatch { found: u32, expected: u32 },
    /// Added for `rusty_multimodal_db`'s `ProductionStore::open_portable`
    /// (`STORAGE-014-FR-005`): the companion record blob
    /// (`<ages path>.records`, see `durability::record_blob`) is missing, isn't a
    /// record blob at all, was written by an incompatible build, or its
    /// body doesn't decode. `cause` names which. Deliberately distinct
    /// from [`Self::InvalidMagic`]/[`Self::SchemaVersionMismatch`], which
    /// describe the *ages* file: a caller who copied only the `.mmap` file
    /// (a pre-`STORAGE-014` backup, say) gets told the companion is the
    /// problem, not misled into thinking the ages file is corrupt. The
    /// `Symmetric` edge-list companion (`<path>.edges`, `STORAGE-016`,
    /// `generic::edge_blob`) reports through this same variant — `path`
    /// says which companion failed.
    #[error("record blob at {path}: {cause}")]
    RecordBlobUnreadable {
        path: std::path::PathBuf,
        cause: String,
    },
    /// Added for the per-field `MmapScanned` layer (`STORAGE-017-FR-009`,
    /// `generic::mmap_scanned`): the slot file at `path` has a valid
    /// header but its slot data isn't a whole number of the slots this
    /// layer's `(Id, V)` pair would write — `body_len` bytes after the
    /// header against a `slot_width`-byte slot. Either the file was
    /// written for a different record shape (another field's, or another
    /// domain's, column in a directory this stack doesn't own) or it was
    /// truncated mid-slot. Detection only, and deliberately weak: a
    /// foreign file whose slot width happens to divide the same body
    /// length passes this check — the tagged record blob, read first on
    /// the portable path, is what catches a foreign directory outright.
    /// `GenericMmapStore` itself does *not* raise this (it ignores a
    /// trailing partial slot, the WAL reader's permissive-truncation
    /// convention); the layer refuses because, having no blob of its own,
    /// this is the only check it has.
    #[error("mmap slot file at {path}: {body_len} bytes of slot data is not a whole number of {slot_width}-byte slots (written for another record shape, or truncated mid-slot)")]
    SlotWidthMismatch {
        path: std::path::PathBuf,
        body_len: usize,
        slot_width: usize,
    },
}

/// `fsync` the directory holding `path`, so a rename or create just made
/// in it is itself on disk — a file's own `sync_all` covers its bytes,
/// never the directory entry that names it (`ADR-0092`, `DDL-FR-004`).
/// Every write-to-temp-then-rename install in this crate calls this
/// after its rename. A `path` with no parent (bare relative name) syncs
/// the current directory. On targets where a directory cannot be opened
/// as a file (Windows) this is a no-op: the rename's visibility there is
/// the filesystem's own promise, and this crate's servers are Linux-only
/// in CI.
pub fn sync_parent_dir(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => std::path::Path::new("."),
        };
        std::fs::File::open(parent)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}
