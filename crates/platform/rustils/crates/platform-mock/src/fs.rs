//! In-memory implementation of the capability-style fs traits.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::sync::{Arc, Mutex};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::fs::{
    AccessMode, AnonymousFile, Dir, DirEntry, File, FileId, FileType, Metadata, Mode, OpenOptions,
    UnixMode,
};

#[derive(Debug, Default)]
enum Node {
    #[default]
    Unreachable,
    File(Vec<u8>),
    Dir(BTreeMap<OsString, Arc<Mutex<Node>>>),
    Symlink(OsString),
}

fn err(kind: ErrorKind, op: &'static str, rel: &OsStr) -> PlatformError {
    PlatformError::new(kind, OsCode::None, op).with_path(rel)
}

/// A directory capability over an in-memory tree.
///
/// `open_dir` hands out capabilities sharing the same underlying nodes, so
/// tests observe writes through any handle — matching real-backend
/// semantics where a `Dir` is a live handle, not a snapshot.
#[derive(Clone)]
pub struct MockDir {
    node: Arc<Mutex<Node>>,
}

impl MockDir {
    /// A new, empty root directory.
    pub fn root() -> Self {
        Self {
            node: Arc::new(Mutex::new(Node::Dir(BTreeMap::new()))),
        }
    }

    /// Convenience for seeding test fixtures.
    pub fn with_file(self, name: impl Into<OsString>, contents: impl Into<Vec<u8>>) -> Self {
        {
            let mut n = crate::sync::lock(&self.node);
            let Node::Dir(entries) = &mut *n else {
                unreachable!("root is a dir")
            };
            entries.insert(
                name.into(),
                Arc::new(Mutex::new(Node::File(contents.into()))),
            );
        }
        self
    }

    fn child(&self, rel: &OsStr, op: &'static str) -> Result<Arc<Mutex<Node>>> {
        let n = crate::sync::lock(&self.node);
        let Node::Dir(entries) = &*n else {
            return Err(err(ErrorKind::NotADirectory, op, rel));
        };
        entries
            .get(rel)
            .cloned()
            .ok_or_else(|| err(ErrorKind::NotFound, op, rel))
    }
}

/// An open handle to an in-memory file. `pos` is `Arc<Mutex<_>>`, not a
/// plain field: [`File::try_clone`] must share it with the clone (a real
/// `dup`/`DuplicateHandle`'s defining property — a read/write through
/// either handle advances the other's next position too), which a
/// bare-`usize` field re-initialized per handle cannot express. Two
/// independent [`Dir::open`] calls on the same path still get two
/// independent `Arc`s (correctly *not* sharing), since `open` always
/// constructs a fresh one — only `try_clone` shares the existing `Arc`.
pub struct MockFile {
    node: Arc<Mutex<Node>>,
    pos: Arc<Mutex<usize>>,
    readable: bool,
    writable: bool,
}

impl File for MockFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if !self.readable {
            return Err(PlatformError::new(
                ErrorKind::PermissionDenied,
                OsCode::None,
                "read",
            ));
        }
        let n = crate::sync::lock(&self.node);
        let Node::File(data) = &*n else {
            return Err(PlatformError::new(
                ErrorKind::IsADirectory,
                OsCode::None,
                "read",
            ));
        };
        let mut pos = crate::sync::lock(&self.pos);
        let remaining = &data[(*pos).min(data.len())..];
        let count = remaining.len().min(buf.len());
        buf[..count].copy_from_slice(&remaining[..count]);
        *pos += count;
        Ok(count)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if !self.writable {
            return Err(PlatformError::new(
                ErrorKind::PermissionDenied,
                OsCode::None,
                "write",
            ));
        }
        let mut n = crate::sync::lock(&self.node);
        let Node::File(data) = &mut *n else {
            return Err(PlatformError::new(
                ErrorKind::IsADirectory,
                OsCode::None,
                "write",
            ));
        };
        data.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn sync_all(&mut self) -> Result<()> {
        // In-memory: writes are already "durable" the instant they land.
        Ok(())
    }

    fn try_clone(&self) -> Result<Box<dyn File>> {
        Ok(Box::new(MockFile {
            node: self.node.clone(),
            // Shared, not re-initialized: see this struct's doc comment.
            pos: self.pos.clone(),
            readable: self.readable,
            writable: self.writable,
        }))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Dir for MockDir {
    fn open(&self, rel: &OsStr, opts: &OpenOptions) -> Result<Box<dyn File>> {
        let existing = self.child(rel, "open");
        let node = match existing {
            Ok(node) => {
                if opts.create_new {
                    return Err(err(ErrorKind::AlreadyExists, "open", rel));
                }
                if opts.truncate {
                    let mut n = crate::sync::lock(&node);
                    if let Node::File(data) = &mut *n {
                        data.clear();
                    }
                }
                node
            }
            Err(e) if opts.create || opts.create_new => {
                let _ = e;
                let node = Arc::new(Mutex::new(Node::File(Vec::new())));
                let mut n = crate::sync::lock(&self.node);
                let Node::Dir(entries) = &mut *n else {
                    return Err(err(ErrorKind::NotADirectory, "open", rel));
                };
                entries.insert(rel.to_os_string(), node.clone());
                node
            }
            Err(e) => return Err(e),
        };
        Ok(Box::new(MockFile {
            node,
            pos: Arc::new(Mutex::new(0)),
            readable: opts.read,
            writable: opts.write || opts.append,
        }))
    }

    fn open_dir(&self, rel: &OsStr) -> Result<Box<dyn Dir>> {
        let node = self.child(rel, "open_dir")?;
        {
            let n = crate::sync::lock(&node);
            if !matches!(&*n, Node::Dir(_)) {
                return Err(err(ErrorKind::NotADirectory, "open_dir", rel));
            }
        }
        Ok(Box::new(MockDir { node }))
    }

    fn create_dir(&self, rel: &OsStr) -> Result<()> {
        let mut n = crate::sync::lock(&self.node);
        let Node::Dir(entries) = &mut *n else {
            return Err(err(ErrorKind::NotADirectory, "create_dir", rel));
        };
        if entries.contains_key(rel) {
            return Err(err(ErrorKind::AlreadyExists, "create_dir", rel));
        }
        entries.insert(
            rel.to_os_string(),
            Arc::new(Mutex::new(Node::Dir(BTreeMap::new()))),
        );
        Ok(())
    }

    fn metadata(&self, rel: &OsStr) -> Result<Metadata> {
        let node = self.child(rel, "metadata")?;
        let n = crate::sync::lock(&node);
        // No hard-link concept in an in-memory tree (every node has
        // exactly one parent entry pointing at it) and no real clock to
        // read a modification time from — `nlink: 1`/`modified:
        // UNIX_EPOCH` are deterministic, honestly-labeled defaults, the
        // same "not real, but reproducible" tradeoff `MockCsprng`'s
        // fixed seed and `unix_mode`'s `UnixMode::default()` already make.
        const NLINK: u64 = 1;
        let modified = std::time::SystemTime::UNIX_EPOCH;
        Ok(match &*n {
            Node::File(data) => Metadata {
                file_type: FileType::File,
                len: data.len() as u64,
                nlink: NLINK,
                modified,
            },
            Node::Dir(_) => Metadata {
                file_type: FileType::Dir,
                len: 0,
                nlink: NLINK,
                modified,
            },
            // Real lstat-style length is the target string's byte
            // length; not asserted cross-backend (the parity suite
            // avoids pinning it — Windows reparse points don't carry
            // the same value), but a real number is more honest here
            // than a placeholder 0.
            Node::Symlink(target) => Metadata {
                file_type: FileType::Symlink,
                len: target.as_encoded_bytes().len() as u64,
                nlink: NLINK,
                modified,
            },
            Node::Unreachable => unreachable!(),
        })
    }

    fn access(&self, rel: &OsStr, mode: AccessMode) -> Result<()> {
        // An empty mode is a vacuous yes, even for a name that doesn't
        // exist — matching both real backends' contract (existence is
        // metadata's job, not this one's).
        if !(mode.read || mode.write || mode.execute) {
            return Ok(());
        }
        // No permission model to probe (every mock entry is equally
        // accessible to everyone) — existence is the only thing this
        // can honestly answer, same limitation `open` already has for
        // symlink-following (never implemented; see `Node::Symlink`).
        self.child(rel, "access").map(drop)
    }

    fn unix_mode(&self, rel: &OsStr) -> Result<Option<UnixMode>> {
        self.child(rel, "unix_mode")?;
        // No real permission model — a deterministic all-default
        // answer, not `None`: `None` means "this OS has no such
        // concept" (Windows's contract), not "not modeled here."
        Ok(Some(UnixMode::default()))
    }

    fn set_unix_mode(&self, rel: &OsStr, _mode: Mode) -> Result<()> {
        // Same "no real permission model" stance as `unix_mode` above:
        // existence is checked and enforced (matching both real
        // backends' `NotFound` contract), but the requested bits are
        // not stored anywhere — `unix_mode` already answers a fixed
        // default regardless of any prior `set_unix_mode` call, so
        // persisting state here would create a round-trip this mock
        // can't actually honor on the read side.
        self.child(rel, "set_unix_mode").map(drop)
    }

    fn file_id(&self, rel: &OsStr) -> Result<FileId> {
        let node = self.child(rel, "file_id")?;
        // The node's own address is a stable, distinct-per-object
        // identity for as long as the mock tree lives — as good an
        // opaque analog for "same underlying file" as a real
        // (dev, ino)/(volume, index) pair.
        let addr = Arc::as_ptr(&node) as usize as u64;
        Ok(FileId(0, addr))
    }

    fn read_dir(&self) -> Result<Vec<DirEntry>> {
        let n = crate::sync::lock(&self.node);
        let Node::Dir(entries) = &*n else {
            return Err(PlatformError::new(
                ErrorKind::NotADirectory,
                OsCode::None,
                "read_dir",
            ));
        };
        Ok(entries
            .iter()
            .map(|(name, node)| {
                let file_type = match &*crate::sync::lock(node) {
                    Node::File(_) => FileType::File,
                    Node::Dir(_) => FileType::Dir,
                    Node::Symlink(_) => FileType::Symlink,
                    Node::Unreachable => unreachable!(),
                };
                DirEntry {
                    name: name.clone(),
                    file_type,
                }
            })
            .collect())
    }

    fn remove_file(&self, rel: &OsStr) -> Result<()> {
        self.remove(rel, false, "remove_file")
    }

    fn remove_dir(&self, rel: &OsStr) -> Result<()> {
        self.remove(rel, true, "remove_dir")
    }

    fn symlink(&self, target: &OsStr, link_name: &OsStr) -> Result<()> {
        let mut n = crate::sync::lock(&self.node);
        let Node::Dir(entries) = &mut *n else {
            return Err(err(ErrorKind::NotADirectory, "symlink", link_name));
        };
        if entries.contains_key(link_name) {
            return Err(err(ErrorKind::AlreadyExists, "symlink", link_name));
        }
        entries.insert(
            link_name.to_os_string(),
            Arc::new(Mutex::new(Node::Symlink(target.to_os_string()))),
        );
        Ok(())
    }

    fn read_link(&self, rel: &OsStr) -> Result<OsString> {
        let node = self.child(rel, "read_link")?;
        let n = crate::sync::lock(&node);
        match &*n {
            Node::Symlink(target) => Ok(target.clone()),
            _ => Err(err(ErrorKind::InvalidInput, "read_link", rel)),
        }
    }

    fn rename(&self, from: &OsStr, to: &OsStr) -> Result<()> {
        self.rename_impl(from, to, true)
    }

    fn rename_no_replace(&self, from: &OsStr, to: &OsStr) -> Result<()> {
        self.rename_impl(from, to, false)
    }
}

impl MockDir {
    fn remove(&self, rel: &OsStr, want_dir: bool, op: &'static str) -> Result<()> {
        let mut n = crate::sync::lock(&self.node);
        let Node::Dir(entries) = &mut *n else {
            return Err(err(ErrorKind::NotADirectory, op, rel));
        };
        let Some(node) = entries.get(rel) else {
            return Err(err(ErrorKind::NotFound, op, rel));
        };
        {
            let child = crate::sync::lock(node);
            match (&*child, want_dir) {
                (Node::File(_), true) => return Err(err(ErrorKind::NotADirectory, op, rel)),
                (Node::Symlink(_), true) => return Err(err(ErrorKind::NotADirectory, op, rel)),
                (Node::Dir(_), false) => return Err(err(ErrorKind::IsADirectory, op, rel)),
                (Node::Dir(entries), true) if !entries.is_empty() => {
                    return Err(err(ErrorKind::DirectoryNotEmpty, op, rel))
                }
                _ => {}
            }
        }
        entries.remove(rel);
        Ok(())
    }

    fn rename_impl(&self, from: &OsStr, to: &OsStr, replace: bool) -> Result<()> {
        let mut n = crate::sync::lock(&self.node);
        let Node::Dir(entries) = &mut *n else {
            return Err(err(ErrorKind::NotADirectory, "rename", from));
        };
        if !entries.contains_key(from) {
            return Err(err(ErrorKind::NotFound, "rename", from));
        }
        if !replace && entries.contains_key(to) {
            return Err(err(ErrorKind::AlreadyExists, "rename", to));
        }
        let Some(node) = entries.remove(from) else {
            return Err(err(ErrorKind::NotFound, "rename", from));
        };
        entries.insert(to.to_os_string(), node);
        Ok(())
    }
}

/// [`AnonymousFile`] over a fresh, freestanding [`Node::File`] — not
/// attached to any [`MockDir`] tree, matching the real backends' own
/// "no directory capability involved at all" contract: unlike every
/// [`MockFile`] [`MockDir::open`] hands out, one from here has no
/// parent entries map any [`MockDir::read_dir`] could ever enumerate
/// it through, and dropping every handle to it (there is no
/// `remove_file` call that could reach it) is the only way it goes
/// away — the same unlinked-from-birth lifetime a real `memfd_create`
/// fd has.
#[derive(Debug, Default)]
pub struct MockAnonymousFile;

impl AnonymousFile for MockAnonymousFile {
    fn create_memfd(&self, _name: &str) -> Result<Box<dyn File>> {
        Ok(Box::new(MockFile {
            node: Arc::new(Mutex::new(Node::File(Vec::new()))),
            pos: Arc::new(Mutex::new(0)),
            readable: true,
            writable: true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_write_then_read() {
        let root = MockDir::root();
        let mut f = root
            .open(OsStr::new("a.txt"), &OpenOptions::create_truncate())
            .expect("create");
        f.write(b"hello").expect("write");

        let mut f = root
            .open(OsStr::new("a.txt"), &OpenOptions::read())
            .expect("open");
        let mut buf = [0u8; 16];
        let n = f.read(&mut buf).expect("read");
        assert_eq!(&buf[..n], b"hello");
    }

    #[test]
    fn try_clone_shares_the_read_position() {
        let root = MockDir::root().with_file("a.txt", *b"0123456789");
        let mut f = root
            .open(OsStr::new("a.txt"), &OpenOptions::read())
            .expect("open");
        let mut clone = f.try_clone().expect("try_clone");

        let mut buf = [0u8; 3];
        assert_eq!(f.read(&mut buf).expect("read via original"), 3);
        assert_eq!(&buf, b"012");

        // The clone's next read must continue from where the ORIGINAL
        // left off — the shared-offset property `dup(2)`/
        // `DuplicateHandle` have and a fresh `open` of the same path
        // does not (this trait method's whole point, per its own doc
        // comment: D5's `2>&1`/`&> file` shape needs it).
        assert_eq!(clone.read(&mut buf).expect("read via clone"), 3);
        assert_eq!(&buf, b"345");

        // And back through the original: still shared, not two
        // independent cursors that happened to start aligned.
        assert_eq!(f.read(&mut buf).expect("read via original again"), 3);
        assert_eq!(&buf, b"678");
    }

    #[test]
    fn capabilities_share_state() {
        let root = MockDir::root();
        root.create_dir(OsStr::new("sub")).expect("mkdir");
        let sub = root.open_dir(OsStr::new("sub")).expect("open_dir");
        sub.open(OsStr::new("f"), &OpenOptions::create_truncate())
            .expect("create");
        // Visible through a second capability to the same node:
        let sub2 = root.open_dir(OsStr::new("sub")).expect("open_dir");
        assert_eq!(sub2.read_dir().expect("read_dir").len(), 1);
    }

    #[test]
    fn non_utf8_names_are_representable() {
        // Pins RFC v2 §5.2: the surface must carry names `str` cannot.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let root = MockDir::root();
            let name = OsStr::from_bytes(b"caf\xe9"); // Latin-1 é — invalid UTF-8
            root.open(name, &OpenOptions::create_truncate())
                .expect("create");
            assert!(root.metadata(name).is_ok());
        }
    }

    #[test]
    fn remove_dir_refuses_non_empty() {
        let root = MockDir::root();
        root.create_dir(OsStr::new("d")).expect("mkdir");
        let d = root.open_dir(OsStr::new("d")).expect("open");
        d.open(OsStr::new("f"), &OpenOptions::create_truncate())
            .expect("create");
        let e = root.remove_dir(OsStr::new("d")).expect_err("must refuse");
        assert_eq!(e.kind, ErrorKind::DirectoryNotEmpty);
    }

    #[test]
    fn rename_replaces_by_default_and_moves_the_same_content() {
        let root = MockDir::root().with_file("a.txt", "hi");
        root.rename(OsStr::new("a.txt"), OsStr::new("b.txt"))
            .expect("rename");
        assert_eq!(
            root.metadata(OsStr::new("a.txt")).unwrap_err().kind,
            ErrorKind::NotFound
        );
        assert_eq!(root.metadata(OsStr::new("b.txt")).unwrap().len, 2);

        // Replaces an existing "c.txt" by default.
        let root = root.with_file("c.txt", "xxxxx");
        root.rename(OsStr::new("b.txt"), OsStr::new("c.txt"))
            .expect("rename replaces");
        assert_eq!(root.metadata(OsStr::new("c.txt")).unwrap().len, 2);
    }

    #[test]
    fn rename_no_replace_refuses_atomically_when_destination_exists() {
        let root = MockDir::root()
            .with_file("a.txt", "hi")
            .with_file("b.txt", "existing");
        let e = root
            .rename_no_replace(OsStr::new("a.txt"), OsStr::new("b.txt"))
            .expect_err("must refuse");
        assert_eq!(e.kind, ErrorKind::AlreadyExists);
        // No partial move: both names are exactly as they were.
        assert_eq!(root.metadata(OsStr::new("a.txt")).unwrap().len, 2);
        assert_eq!(root.metadata(OsStr::new("b.txt")).unwrap().len, 8);

        root.rename_no_replace(OsStr::new("a.txt"), OsStr::new("c.txt"))
            .expect("no existing destination: must succeed");
        assert_eq!(root.metadata(OsStr::new("c.txt")).unwrap().len, 2);
    }

    #[test]
    fn symlink_stores_target_verbatim_and_refuses_to_clobber() {
        let root = MockDir::root().with_file("real.txt", "hi");
        root.symlink(OsStr::new("nowhere/dangling"), OsStr::new("link"))
            .expect("symlink");
        assert_eq!(
            root.metadata(OsStr::new("link")).unwrap().file_type,
            FileType::Symlink
        );
        assert_eq!(
            root.read_link(OsStr::new("link")).unwrap(),
            OsStr::new("nowhere/dangling").to_os_string()
        );

        let e = root
            .symlink(OsStr::new("real.txt"), OsStr::new("link"))
            .expect_err("must refuse: link already exists");
        assert_eq!(e.kind, ErrorKind::AlreadyExists);

        let e = root
            .read_link(OsStr::new("real.txt"))
            .expect_err("not a symlink");
        assert_eq!(e.kind, ErrorKind::InvalidInput);
    }

    #[test]
    fn remove_dir_refuses_a_symlink_even_to_a_directory() {
        let root = MockDir::root();
        root.create_dir(OsStr::new("d")).expect("mkdir");
        root.symlink(OsStr::new("d"), OsStr::new("dlink"))
            .expect("symlink");
        // A symlink is never itself a directory, regardless of what it
        // points at — matching POSIX `rmdir` refusing a symlink with
        // `ENOTDIR`.
        let e = root
            .remove_dir(OsStr::new("dlink"))
            .expect_err("must refuse");
        assert_eq!(e.kind, ErrorKind::NotADirectory);
        root.remove_file(OsStr::new("dlink")).expect("rm dlink");
    }

    #[test]
    fn access_checks_existence_only_no_permission_model() {
        let root = MockDir::root().with_file("f.txt", "hi");
        root.access(OsStr::new("f.txt"), AccessMode::read())
            .expect("exists, mock grants everything");
        root.access(
            OsStr::new("f.txt"),
            AccessMode {
                read: true,
                write: true,
                execute: true,
            },
        )
        .expect("mock has no permission model to deny any of these");
        let e = root
            .access(OsStr::new("missing"), AccessMode::read())
            .expect_err("must fail: missing");
        assert_eq!(e.kind, ErrorKind::NotFound);
        // An empty mode is a vacuous yes, even for a name that doesn't
        // exist.
        root.access(OsStr::new("also-missing"), AccessMode::default())
            .expect("empty mode never fails");
    }

    #[test]
    fn unix_mode_is_a_deterministic_default_not_none() {
        let root = MockDir::root().with_file("f.txt", "hi");
        // `None` would mean "this OS has no such concept" (Windows's
        // contract); the mock always answers `Some`, just with no real
        // permission model behind it.
        assert_eq!(
            root.unix_mode(OsStr::new("f.txt")).unwrap(),
            Some(UnixMode::default())
        );
        assert_eq!(
            root.unix_mode(OsStr::new("missing")).unwrap_err().kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn set_unix_mode_succeeds_but_does_not_change_the_deterministic_default() {
        let root = MockDir::root().with_file("f.txt", "hi");
        let requested = Mode {
            setuid: true,
            permissions: 0o700,
            ..Mode::default()
        };
        root.set_unix_mode(OsStr::new("f.txt"), requested)
            .expect("no real permission model to reject this on");
        // `unix_mode` still answers its fixed default — `set_unix_mode`
        // has nothing to persist into, same as `unix_mode`'s own
        // doc comment already states.
        assert_eq!(
            root.unix_mode(OsStr::new("f.txt")).unwrap(),
            Some(UnixMode::default())
        );
        assert_eq!(
            root.set_unix_mode(OsStr::new("missing"), Mode::default())
                .unwrap_err()
                .kind,
            ErrorKind::NotFound
        );
    }

    #[test]
    fn file_id_is_stable_per_entry_and_distinct_across_entries() {
        let root = MockDir::root()
            .with_file("a.txt", "one")
            .with_file("b.txt", "two");
        let id_a = root.file_id(OsStr::new("a.txt")).unwrap();
        let id_a_again = root.file_id(OsStr::new("a.txt")).unwrap();
        assert_eq!(id_a, id_a_again);
        let id_b = root.file_id(OsStr::new("b.txt")).unwrap();
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn write_atomic_publishes_and_leaves_no_temp_file() {
        let root = MockDir::root();
        root.write_atomic(OsStr::new("f.txt"), b"first")
            .expect("write_atomic");
        let mut f = root
            .open(OsStr::new("f.txt"), &OpenOptions::read())
            .expect("open");
        let mut buf = [0u8; 16];
        let n = f.read(&mut buf).expect("read");
        assert_eq!(&buf[..n], b"first");
        drop(f);

        // A second call fully overwrites — no residual bytes from the
        // shorter old content, no leftover temp-name entry.
        root.write_atomic(OsStr::new("f.txt"), b"2")
            .expect("write_atomic 2");
        let names: Vec<_> = root
            .read_dir()
            .expect("read_dir")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec![OsStr::new("f.txt").to_os_string()]);
        assert_eq!(root.metadata(OsStr::new("f.txt")).unwrap().len, 1);
    }

    #[test]
    fn anonymous_file_round_trips_and_is_independent_of_any_dir() {
        let anon = MockAnonymousFile;
        let mut f = anon.create_memfd("scratch").expect("create_memfd");
        let n = f.write(b"hello").expect("write");
        assert_eq!(n, 5);
        let mut buf = [0u8; 5];
        let n = f.read(&mut buf).expect("read");
        assert_eq!(&buf[..n], b"hello");

        // Two independent calls never alias: unlike `MockDir::open`ing
        // the same `rel` twice (which resolves to the same underlying
        // node), `create_memfd` has no name/path to collide on at all.
        let mut other = anon.create_memfd("scratch").expect("create_memfd");
        let n = other.read(&mut buf).expect("read from the other instance");
        assert_eq!(n, 0, "a second create_memfd call must start empty");
    }
}
