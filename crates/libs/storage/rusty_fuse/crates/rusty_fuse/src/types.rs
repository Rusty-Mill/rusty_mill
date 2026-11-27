//! Plain data types of the port.

use std::num::NonZeroU64;
use std::time::{Duration, SystemTime};

use crate::Name;

/// A node's id, stable for the life of a mount. The root is [`NodeId::ROOT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(NonZeroU64);

impl NodeId {
    /// The root directory.
    pub const ROOT: Self = Self(NonZeroU64::MIN);

    /// Wraps a raw id; `None` for zero, which no node may use.
    #[must_use]
    pub const fn new(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(id) => Some(Self(id)),
            None => None,
        }
    }

    /// The raw id.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link; its target comes from `readlink`.
    Symlink,
}

/// A node's attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    /// What the node is.
    pub kind: NodeKind,
    /// Size in bytes (the target length for a symlink).
    pub size: u64,
    /// Permission bits, `0o7777` at most.
    pub perm: u16,
    /// Hard-link count.
    pub nlink: u32,
    /// Owner user id (0 where the OS has none).
    pub uid: u32,
    /// Owner group id (0 where the OS has none).
    pub gid: u32,
    /// Last content change.
    pub mtime: SystemTime,
    /// Last metadata change (last content change where the OS has none).
    pub ctime: SystemTime,
}

/// How long an adapter may trust a name or attributes. Zero is always correct.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ttl(pub Duration);

impl Ttl {
    /// Do not cache.
    pub const ZERO: Self = Self(Duration::ZERO);
}

/// The answer to `lookup`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The node found.
    pub id: NodeId,
    /// Bumped when an id is reused for a different node.
    pub generation: u64,
    /// The node's attributes.
    pub attr: Attr,
    /// How long the name and attributes may be cached.
    pub ttl: Ttl,
}

/// A position in a directory listing. [`Cookie::START`] begins a listing; each
/// entry carries the cookie that resumes after it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cookie(pub u64);

impl Cookie {
    /// The start of a listing.
    pub const START: Self = Self(0);
}

/// Identifies one version of a directory's contents; it changes when the
/// directory does, which invalidates outstanding cookies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Verifier(pub u64);

impl Verifier {
    /// What a caller passes when it holds no verifier yet.
    pub const NONE: Self = Self(0);
}

/// One directory entry pushed into a [`DirSink`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry<'a> {
    /// The entry's node.
    pub id: NodeId,
    /// What the entry is.
    pub kind: NodeKind,
    /// The entry's name.
    pub name: Name<'a>,
    /// The cookie that resumes the listing after this entry.
    pub next: Cookie,
}

/// Receives directory entries from `readdir`.
pub trait DirSink {
    /// Takes one entry; returns `false` when full, in which case the entry was
    /// not taken and the backend stops.
    fn push(&mut self, entry: DirEntry<'_>) -> bool;
}

/// A backend's per-open state. [`Handle::NONE`] means "no state".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Handle(pub u64);

impl Handle {
    /// No per-open state.
    pub const NONE: Self = Self(0);
}

/// How a node is opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OpenFlags {
    /// The caller wants to write.
    pub write: bool,
}

impl OpenFlags {
    /// Read-only access.
    pub const READ: Self = Self { write: false };
    /// Read-write access.
    pub const WRITE: Self = Self { write: true };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_zero_is_not_a_node() {
        assert_eq!(NodeId::new(0), None);
        assert_eq!(NodeId::new(1), Some(NodeId::ROOT));
        assert_eq!(NodeId::ROOT.get(), 1);
    }
}
