//! The path-to-node table: which relative path each `NodeId` names.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusty_fuse::NodeId;

#[derive(Debug)]
struct Record {
    rel: PathBuf,
    lookups: u64,
}

/// Maps node ids to paths relative to the mirrored root and back.
///
/// Ids are handed out in order and never reused, so a generation of 0 is
/// always correct. A node stays while it holds lookup references; nodes seen
/// only through `readdir` hold none and stay until a `forget` reaches them.
#[derive(Debug)]
pub(crate) struct NodeTable {
    by_id: HashMap<NodeId, Record>,
    by_path: HashMap<PathBuf, NodeId>,
    next: u64,
}

impl NodeTable {
    /// A table holding only the root, at the empty relative path.
    pub(crate) fn new() -> Self {
        let mut table = Self {
            by_id: HashMap::new(),
            by_path: HashMap::new(),
            next: 2,
        };
        table.by_id.insert(
            NodeId::ROOT,
            Record {
                rel: PathBuf::new(),
                lookups: 1,
            },
        );
        table.by_path.insert(PathBuf::new(), NodeId::ROOT);
        table
    }

    /// The relative path of `id`, if known.
    pub(crate) fn path(&self, id: NodeId) -> Option<&Path> {
        self.by_id.get(&id).map(|r| r.rel.as_path())
    }

    /// The id for `rel`, created if new, without taking a lookup reference.
    pub(crate) fn intern(&mut self, rel: PathBuf) -> NodeId {
        if let Some(&id) = self.by_path.get(&rel) {
            return id;
        }
        let id = NodeId::new(self.next).unwrap_or(NodeId::ROOT);
        self.next += 1;
        self.by_path.insert(rel.clone(), id);
        self.by_id.insert(id, Record { rel, lookups: 0 });
        id
    }

    /// The id for `rel`, created if new, taking one lookup reference.
    pub(crate) fn lookup(&mut self, rel: PathBuf) -> NodeId {
        let id = self.intern(rel);
        if let Some(record) = self.by_id.get_mut(&id) {
            record.lookups += 1;
        }
        id
    }

    /// Drops `nlookup` references; a non-root node with none left is removed.
    pub(crate) fn forget(&mut self, id: NodeId, nlookup: u64) {
        if id == NodeId::ROOT {
            return;
        }
        let Some(record) = self.by_id.get_mut(&id) else {
            return;
        };
        record.lookups = record.lookups.saturating_sub(nlookup);
        if record.lookups == 0 {
            if let Some(record) = self.by_id.remove(&id) {
                self.by_path.remove(&record.rel);
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_id.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_present_and_never_forgotten() {
        let mut table = NodeTable::new();
        assert_eq!(table.path(NodeId::ROOT), Some(Path::new("")));
        table.forget(NodeId::ROOT, u64::MAX);
        assert_eq!(table.path(NodeId::ROOT), Some(Path::new("")));
    }

    #[test]
    fn same_path_same_id_and_new_paths_get_new_ids() {
        let mut table = NodeTable::new();
        let a = table.lookup(PathBuf::from("a"));
        assert_eq!(table.intern(PathBuf::from("a")), a);
        let b = table.intern(PathBuf::from("b"));
        assert_ne!(a, b);
        assert_ne!(a, NodeId::ROOT);
        assert_eq!(table.path(b), Some(Path::new("b")));
    }

    #[test]
    fn forget_removes_only_after_the_last_reference() {
        let mut table = NodeTable::new();
        let a = table.lookup(PathBuf::from("a"));
        table.lookup(PathBuf::from("a"));
        table.forget(a, 1);
        assert!(table.path(a).is_some(), "one reference left");
        table.forget(a, 1);
        assert!(table.path(a).is_none(), "removed at zero");
        assert_eq!(table.len(), 1);
        let again = table.lookup(PathBuf::from("a"));
        assert_ne!(again, a, "ids are never reused");
    }

    #[test]
    fn forgetting_unknown_ids_is_harmless() {
        let mut table = NodeTable::new();
        table.forget(NodeId::new(99).unwrap_or(NodeId::ROOT), 1);
        assert_eq!(table.len(), 1);
    }
}
