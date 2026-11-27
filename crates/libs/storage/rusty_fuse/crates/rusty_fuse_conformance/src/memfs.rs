//! `MemFs`: an in-memory backend for port tests.

use std::collections::BTreeMap;
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::SystemTime;

use rusty_fuse::{
    Attr, Cookie, DirEntry, DirSink, Entry, Errno, Filesystem, Handle, Name, NodeId, NodeKind, Ttl,
    Verifier,
};

use crate::FixtureItem;

#[derive(Debug)]
struct Node {
    kind: NodeKind,
    /// File contents, or a symlink's target.
    data: Vec<u8>,
    /// Children by name; directories only.
    children: BTreeMap<Vec<u8>, NodeId>,
    /// Bumped on every change to `children`; never zero.
    version: u64,
    mtime: SystemTime,
}

impl Node {
    fn new(kind: NodeKind, data: Vec<u8>) -> Self {
        Self {
            kind,
            data,
            children: BTreeMap::new(),
            version: 1,
            mtime: SystemTime::now(),
        }
    }

    fn attr(&self) -> Attr {
        let (perm, nlink) = match self.kind {
            NodeKind::Dir => (0o755, 2),
            NodeKind::File => (0o644, 1),
            NodeKind::Symlink => (0o777, 1),
        };
        Attr {
            kind: self.kind,
            size: self.data.len() as u64,
            perm,
            nlink,
            uid: 0,
            gid: 0,
            mtime: self.mtime,
            ctime: self.mtime,
        }
    }
}

/// An in-memory backend. Built with `add_*`; served read-only through the port.
///
/// Node ids are indexes into a vector plus one, never reused; `forget` is a
/// no-op because nothing is ever freed.
#[derive(Debug)]
pub struct MemFs {
    nodes: RwLock<Vec<Node>>,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl MemFs {
    /// An empty filesystem holding only the root directory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(vec![Node::new(NodeKind::Dir, Vec::new())]),
        }
    }

    /// A filesystem loaded with [`crate::fixture`].
    ///
    /// # Errors
    /// Only if the fixture itself is malformed.
    pub fn with_fixture() -> Result<Self, Errno> {
        let fs = Self::new();
        for item in crate::fixture() {
            match item {
                FixtureItem::Dir(path) => {
                    let (parent, name) = fs.resolve_parent(path)?;
                    fs.add_dir(parent, Name::try_from(name)?)?;
                }
                FixtureItem::File(path, data) => {
                    let (parent, name) = fs.resolve_parent(path)?;
                    fs.add_file(parent, Name::try_from(name)?, data)?;
                }
            }
        }
        Ok(fs)
    }

    /// Adds an empty directory.
    ///
    /// # Errors
    /// `NotDir` or `Stale` for a bad parent; `Inval` when the name is taken.
    pub fn add_dir(&self, parent: NodeId, name: Name<'_>) -> Result<NodeId, Errno> {
        self.insert(parent, name, Node::new(NodeKind::Dir, Vec::new()))
    }

    /// Adds a file with `data`.
    ///
    /// # Errors
    /// As [`MemFs::add_dir`].
    pub fn add_file(&self, parent: NodeId, name: Name<'_>, data: Vec<u8>) -> Result<NodeId, Errno> {
        self.insert(parent, name, Node::new(NodeKind::File, data))
    }

    /// Adds a symlink pointing at `target`.
    ///
    /// # Errors
    /// As [`MemFs::add_dir`].
    pub fn add_symlink(
        &self,
        parent: NodeId,
        name: Name<'_>,
        target: Vec<u8>,
    ) -> Result<NodeId, Errno> {
        self.insert(parent, name, Node::new(NodeKind::Symlink, target))
    }

    /// Removes `name` from `parent` (the node itself stays, so its id stays
    /// valid for getattr). Used to test verifier changes.
    ///
    /// # Errors
    /// `NoEnt` when absent, plus the `add_dir` errors.
    pub fn unlink(&self, parent: NodeId, name: Name<'_>) -> Result<(), Errno> {
        let mut nodes = self.write()?;
        let dir = dir_mut(&mut nodes, parent)?;
        dir.children.remove(name.as_bytes()).ok_or(Errno::NoEnt)?;
        dir.version += 1;
        Ok(())
    }

    fn insert(&self, parent: NodeId, name: Name<'_>, node: Node) -> Result<NodeId, Errno> {
        let mut nodes = self.write()?;
        let id = NodeId::new(nodes.len() as u64 + 1).ok_or(Errno::Io)?;
        let dir = dir_mut(&mut nodes, parent)?;
        if dir.children.contains_key(name.as_bytes()) {
            return Err(Errno::Inval);
        }
        dir.children.insert(name.as_bytes().to_vec(), id);
        dir.version += 1;
        nodes.push(node);
        Ok(id)
    }

    fn resolve_parent<'p>(&self, path: &'p str) -> Result<(NodeId, &'p str), Errno> {
        let (dirs, name) = path.rsplit_once('/').unwrap_or(("", path));
        let mut at = NodeId::ROOT;
        for step in dirs.split('/').filter(|s| !s.is_empty()) {
            at = self.lookup(at, Name::try_from(step)?)?.id;
        }
        Ok((at, name))
    }

    fn read_lock(&self) -> Result<RwLockReadGuard<'_, Vec<Node>>, Errno> {
        self.nodes.read().map_err(|_| Errno::Io)
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, Vec<Node>>, Errno> {
        self.nodes.write().map_err(|_| Errno::Io)
    }
}

fn node(nodes: &[Node], id: NodeId) -> Result<&Node, Errno> {
    usize::try_from(id.get() - 1)
        .ok()
        .and_then(|i| nodes.get(i))
        .ok_or(Errno::Stale)
}

fn dir_mut(nodes: &mut [Node], id: NodeId) -> Result<&mut Node, Errno> {
    let index = usize::try_from(id.get() - 1).map_err(|_| Errno::Stale)?;
    let dir = nodes.get_mut(index).ok_or(Errno::Stale)?;
    if dir.kind != NodeKind::Dir {
        return Err(Errno::NotDir);
    }
    Ok(dir)
}

impl Filesystem for MemFs {
    fn lookup(&self, parent: NodeId, name: Name<'_>) -> Result<Entry, Errno> {
        let nodes = self.read_lock()?;
        let dir = node(&nodes, parent)?;
        if dir.kind != NodeKind::Dir {
            return Err(Errno::NotDir);
        }
        let id = *dir.children.get(name.as_bytes()).ok_or(Errno::NoEnt)?;
        let attr = node(&nodes, id)?.attr();
        Ok(Entry {
            id,
            generation: 0,
            attr,
            ttl: Ttl::ZERO,
        })
    }

    fn getattr(&self, id: NodeId) -> Result<Attr, Errno> {
        Ok(node(&self.read_lock()?, id)?.attr())
    }

    fn readdir(
        &self,
        dir: NodeId,
        cookie: Cookie,
        verifier: Verifier,
        sink: &mut dyn DirSink,
    ) -> Result<Verifier, Errno> {
        let nodes = self.read_lock()?;
        let dir = node(&nodes, dir)?;
        if dir.kind != NodeKind::Dir {
            return Err(Errno::NotDir);
        }
        let current = Verifier(dir.version);
        if cookie != Cookie::START && verifier != current {
            return Err(Errno::Stale);
        }
        let skip = usize::try_from(cookie.0).map_err(|_| Errno::Inval)?;
        for (index, (name, &id)) in dir.children.iter().enumerate().skip(skip) {
            let entry = DirEntry {
                id,
                kind: node(&nodes, id)?.kind,
                name: Name::new(name)?,
                next: Cookie(index as u64 + 1),
            };
            if !sink.push(entry) {
                break;
            }
        }
        Ok(current)
    }

    fn read(&self, id: NodeId, _fh: Handle, offset: u64, buf: &mut [u8]) -> Result<usize, Errno> {
        let nodes = self.read_lock()?;
        let file = node(&nodes, id)?;
        match file.kind {
            NodeKind::File => {}
            NodeKind::Dir => return Err(Errno::IsDir),
            NodeKind::Symlink => return Err(Errno::Inval),
        }
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(file.data.len());
        let len = buf.len().min(file.data.len() - start);
        buf[..len].copy_from_slice(&file.data[start..start + len]);
        Ok(len)
    }

    fn readlink(&self, id: NodeId, buf: &mut Vec<u8>) -> Result<(), Errno> {
        let nodes = self.read_lock()?;
        let link = node(&nodes, id)?;
        if link.kind != NodeKind::Symlink {
            return Err(Errno::Inval);
        }
        buf.clear();
        buf.extend_from_slice(&link.data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{assert_fixture, assert_symlink, collect_dir};

    fn name(s: &str) -> Name<'_> {
        Name::try_from(s).expect("valid test name")
    }

    #[test]
    fn memfs_passes_the_shared_assertions() {
        let fs = MemFs::with_fixture().expect("fixture loads");
        assert_fixture(&fs);
    }

    #[test]
    fn memfs_serves_symlinks_without_following_them() {
        let fs = MemFs::with_fixture().expect("fixture loads");
        fs.add_symlink(NodeId::ROOT, name("link"), b"dir/a.txt".to_vec())
            .expect("add link");
        assert_symlink(&fs, "link", b"dir/a.txt");
    }

    #[test]
    fn duplicate_names_are_refused() {
        let fs = MemFs::new();
        fs.add_dir(NodeId::ROOT, name("x")).expect("first add");
        assert_eq!(
            fs.add_file(NodeId::ROOT, name("x"), Vec::new()),
            Err(Errno::Inval)
        );
    }

    #[test]
    fn adding_under_a_file_is_not_dir() {
        let fs = MemFs::new();
        let file = fs
            .add_file(NodeId::ROOT, name("f"), Vec::new())
            .expect("add file");
        assert_eq!(fs.add_dir(file, name("x")), Err(Errno::NotDir));
    }

    #[test]
    fn changed_directory_invalidates_cookies() {
        let fs = MemFs::with_fixture().expect("fixture loads");
        let (_, verifier) = collect_dir(&fs, NodeId::ROOT);
        fs.unlink(NodeId::ROOT, name("hello.txt")).expect("unlink");
        let mut sink = crate::CappedSink::new(8);
        let resumed = fs.readdir(NodeId::ROOT, Cookie(1), verifier, &mut sink);
        assert_eq!(resumed, Err(Errno::Stale));
        let (names, fresh) = collect_dir(&fs, NodeId::ROOT);
        assert_ne!(fresh, verifier);
        assert_eq!(names, vec![b"dir".to_vec()]);
    }
}
