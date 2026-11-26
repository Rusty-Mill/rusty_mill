//! A read-only rusty_fuse backend that mirrors a plain host directory.
//!
//! [`PassthroughFs`] serves an ordinary folder (NTFS on Windows, ext4 or APFS
//! elsewhere) through the [`rusty_fuse::Filesystem`] port. It needs no ProjFS,
//! FUSE or FSKit: it is the reference backend that lets the port be exercised
//! on any machine, and what an adapter mounts once one exists (DECISIONS D22,
//! D23).
//!
//! # Containment
//! - Names are validated as one plain path component, so `..`, separators,
//!   drive prefixes and (on Windows) `:` streams are refused.
//! - Metadata is read with `symlink_metadata`. Symlinks (and Windows
//!   junctions) are served as [`NodeKind::Symlink`] nodes whose target comes
//!   from `readlink`; they are never followed while resolving a lookup or
//!   opening a file, and every ancestor is re-checked before each access.
//!
//! These checks run against the live tree, so a local process that swaps a
//! directory for a symlink between the check and the open can still race them.
//! The mirror is a convenience, not a security boundary against hostile local
//! writers to the source folder.
//!
//! # Limits
//! - Read-only: `open` for writing fails with [`Errno::ReadOnly`]; there is no
//!   `Writable` yet (D9).
//! - FIFOs, sockets and devices are not served: `lookup` answers
//!   `NotSupported` and `readdir` skips them.
//! - On Windows, names that are not valid Unicode are skipped by `readdir`,
//!   and lookups are as case-insensitive as NTFS: `HELLO.TXT` and `hello.txt`
//!   resolve, but to two different node ids.
//! - Node identity is by path: a file replaced on disk keeps its id.

#![forbid(unsafe_code)]

mod host;
mod table;

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rusty_fuse::{
    Attr, Cookie, DirEntry, DirSink, Entry, Errno, Filesystem, Handle, Name, NodeId, NodeKind,
    OpenFlags, Ttl, Verifier,
};

use table::NodeTable;

/// Open files by handle number, each with the node it was opened for.
type HandleMap = HashMap<u64, (NodeId, Arc<File>)>;

/// One listed entry: name bytes, kind, and the host name to join paths with.
type Listed = (Vec<u8>, NodeKind, std::ffi::OsString);

/// A read-only mirror of one host directory.
#[derive(Debug)]
pub struct PassthroughFs {
    root: PathBuf,
    nodes: Mutex<NodeTable>,
    handles: Mutex<HandleMap>,
    next_handle: AtomicU64,
}

impl PassthroughFs {
    /// Mirrors the directory at `root`, which is resolved to an absolute path
    /// once, here.
    ///
    /// # Errors
    /// `NoEnt` when `root` does not exist, `NotDir` when it is not a directory,
    /// otherwise the mapped I/O error.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, Errno> {
        let root = fs::canonicalize(root.as_ref()).map_err(|e| host::errno(&e))?;
        let meta = fs::metadata(&root).map_err(|e| host::errno(&e))?;
        if !meta.is_dir() {
            return Err(Errno::NotDir);
        }
        Ok(Self {
            root,
            nodes: Mutex::new(NodeTable::new()),
            handles: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
        })
    }

    /// The mirrored directory, as resolved by [`PassthroughFs::new`].
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn nodes(&self) -> Result<MutexGuard<'_, NodeTable>, Errno> {
        self.nodes.lock().map_err(|_| Errno::Io)
    }

    fn handles(&self) -> Result<MutexGuard<'_, HandleMap>, Errno> {
        self.handles.lock().map_err(|_| Errno::Io)
    }

    /// The relative path of `id`.
    fn rel(&self, id: NodeId) -> Result<PathBuf, Errno> {
        self.nodes()?
            .path(id)
            .map(Path::to_path_buf)
            .ok_or(Errno::Stale)
    }

    /// The host path of `rel` after checking that no ancestor below the root
    /// is a symlink, plus the node's own metadata, not followed.
    fn stat(&self, rel: &Path) -> Result<(PathBuf, fs::Metadata, NodeKind), Errno> {
        let mut at = self.root.clone();
        let mut parts = rel.components().peekable();
        while let Some(part) = parts.next() {
            at.push(part);
            if parts.peek().is_some() {
                let meta = fs::symlink_metadata(&at).map_err(|e| host::errno(&e))?;
                if !meta.is_dir() || meta.file_type().is_symlink() {
                    return Err(Errno::NotDir);
                }
            }
        }
        let meta = fs::symlink_metadata(&at).map_err(|e| host::errno(&e))?;
        let kind = host::kind(&meta).ok_or(Errno::NotSupported)?;
        Ok((at, meta, kind))
    }

    /// Opens the regular file `id` for reading, without following symlinks.
    fn open_file(&self, id: NodeId) -> Result<File, Errno> {
        let (path, _, kind) = self.stat(&self.rel(id)?)?;
        match kind {
            NodeKind::File => File::open(path).map_err(|e| host::errno(&e)),
            NodeKind::Dir => Err(Errno::IsDir),
            NodeKind::Symlink => Err(Errno::Inval),
        }
    }

    fn dir_rel(&self, dir: NodeId) -> Result<PathBuf, Errno> {
        let rel = self.rel(dir)?;
        match self.stat(&rel)?.2 {
            NodeKind::Dir => Ok(rel),
            NodeKind::File | NodeKind::Symlink => Err(Errno::NotDir),
        }
    }
}

/// A host directory listing, sorted by name bytes so cookies are stable.
fn list(path: &Path) -> Result<Vec<Listed>, Errno> {
    let mut entries = Vec::new();
    for item in fs::read_dir(path).map_err(|e| host::errno(&e))? {
        let item = item.map_err(|e| host::errno(&e))?;
        let os = item.file_name();
        let Some(bytes) = host::name_bytes(&os) else {
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(item.path()) else {
            continue;
        };
        let Some(kind) = host::kind(&meta) else {
            continue;
        };
        entries.push((bytes, kind, os));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

/// FNV-1a over the sorted names and kinds; never [`Verifier::NONE`].
fn verifier(entries: &[Listed]) -> Verifier {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, kind, _) in entries {
        let tag = match kind {
            NodeKind::File => 1u8,
            NodeKind::Dir => 2,
            NodeKind::Symlink => 3,
        };
        for &byte in name.iter().chain([0, tag].iter()) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    Verifier(hash.max(1))
}

impl Filesystem for PassthroughFs {
    fn lookup(&self, parent: NodeId, name: Name<'_>) -> Result<Entry, Errno> {
        let rel = self.dir_rel(parent)?.join(host::component(name)?);
        let (_, meta, kind) = self.stat(&rel)?;
        let id = self.nodes()?.lookup(rel);
        Ok(Entry {
            id,
            generation: 0,
            attr: host::attr(&meta, kind),
            ttl: Ttl::ZERO,
        })
    }

    fn getattr(&self, id: NodeId) -> Result<Attr, Errno> {
        let (_, meta, kind) = self.stat(&self.rel(id)?)?;
        Ok(host::attr(&meta, kind))
    }

    fn readdir(
        &self,
        dir: NodeId,
        cookie: Cookie,
        verifier_in: Verifier,
        sink: &mut dyn DirSink,
    ) -> Result<Verifier, Errno> {
        let rel = self.dir_rel(dir)?;
        let entries = list(&self.root.join(&rel))?;
        let current = verifier(&entries);
        if cookie != Cookie::START && verifier_in != current {
            return Err(Errno::Stale);
        }
        let skip = usize::try_from(cookie.0).map_err(|_| Errno::Inval)?;
        for (index, (bytes, kind, os)) in entries.iter().enumerate().skip(skip) {
            let Ok(name) = Name::new(bytes) else { continue };
            let id = self.nodes()?.intern(rel.join(os));
            let entry = DirEntry {
                id,
                kind: *kind,
                name,
                next: Cookie(index as u64 + 1),
            };
            if !sink.push(entry) {
                break;
            }
        }
        Ok(current)
    }

    fn open(&self, id: NodeId, flags: OpenFlags) -> Result<Handle, Errno> {
        if flags.write {
            return Err(Errno::ReadOnly);
        }
        let file = Arc::new(self.open_file(id)?);
        let fh = self.next_handle.fetch_add(1, Ordering::Relaxed);
        self.handles()?.insert(fh, (id, file));
        Ok(Handle(fh))
    }

    fn read(&self, id: NodeId, fh: Handle, offset: u64, buf: &mut [u8]) -> Result<usize, Errno> {
        let file = if fh == Handle::NONE {
            Arc::new(self.open_file(id)?)
        } else {
            let handles = self.handles()?;
            match handles.get(&fh.0) {
                Some((owner, file)) if *owner == id => Arc::clone(file),
                _ => return Err(Errno::BadHandle),
            }
        };
        host::read_at(&file, offset, buf).map_err(|e| host::errno(&e))
    }

    // `release` and `forget` cannot report errors; a lock is only poisoned
    // after a panic elsewhere, which already surfaced, so they do nothing then.
    fn release(&self, _id: NodeId, fh: Handle) {
        if let Ok(mut handles) = self.handles() {
            handles.remove(&fh.0);
        }
    }

    fn readlink(&self, id: NodeId, buf: &mut Vec<u8>) -> Result<(), Errno> {
        let (path, _, kind) = self.stat(&self.rel(id)?)?;
        if kind != NodeKind::Symlink {
            return Err(Errno::Inval);
        }
        let target = fs::read_link(path).map_err(|e| host::errno(&e))?;
        buf.clear();
        buf.extend_from_slice(target.as_os_str().as_encoded_bytes());
        Ok(())
    }

    fn forget(&self, id: NodeId, nlookup: u64) {
        if let Ok(mut nodes) = self.nodes() {
            nodes.forget(id, nlookup);
        }
    }
}
