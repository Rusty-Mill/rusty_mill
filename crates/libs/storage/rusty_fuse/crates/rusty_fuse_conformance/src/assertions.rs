//! Assertions every backend loaded with [`crate::fixture`] must pass.

use std::collections::BTreeSet;

use rusty_fuse::{
    Cookie, DirEntry, DirSink, Entry, Errno, Filesystem, Handle, Name, NodeId, NodeKind, OpenFlags,
    Verifier,
};

use crate::{big_contents, BIG_LEN, HELLO};

/// A [`DirSink`] that takes at most `capacity` entries, recording name, id,
/// kind and resume cookie of each.
#[derive(Debug, Default)]
pub struct CappedSink {
    capacity: usize,
    /// Entries taken, as (name, id, kind, next cookie).
    pub entries: Vec<(Vec<u8>, NodeId, NodeKind, Cookie)>,
}

impl CappedSink {
    /// A sink that takes at most `capacity` entries.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Vec::new(),
        }
    }
}

impl DirSink for CappedSink {
    fn push(&mut self, entry: DirEntry<'_>) -> bool {
        if self.entries.len() >= self.capacity {
            return false;
        }
        self.entries.push((
            entry.name.as_bytes().to_vec(),
            entry.id,
            entry.kind,
            entry.next,
        ));
        true
    }
}

/// Lists `dir` in one call and returns its names in listing order plus the
/// verifier.
///
/// # Panics
/// When `readdir` fails.
pub fn collect_dir<F: Filesystem + ?Sized>(fs: &F, dir: NodeId) -> (Vec<Vec<u8>>, Verifier) {
    let mut sink = CappedSink::new(usize::MAX);
    let verifier = fs
        .readdir(dir, Cookie::START, Verifier::NONE, &mut sink)
        .expect("readdir from start");
    (
        sink.entries.into_iter().map(|(name, ..)| name).collect(),
        verifier,
    )
}

/// Runs every shared assertion against `fs`, which must hold [`crate::fixture`].
///
/// # Panics
/// On the first assertion that fails.
pub fn assert_fixture<F: Filesystem + ?Sized>(fs: &F) {
    root_is_a_directory(fs);
    lookup_resolves_kinds_and_sizes(fs);
    lookup_failures(fs);
    ids_are_stable(fs);
    reads_whole_ranged_and_past_end(fs);
    reads_through_an_open_handle(fs);
    readdir_lists_each_directory(fs);
    readdir_resumes_from_every_cookie(fs);
    readdir_and_read_reject_the_wrong_kind(fs);
}

/// Asserts that `name` in the root is a symlink whose target is `target`, and
/// that it is not followed by `lookup`.
///
/// # Panics
/// When it is not.
pub fn assert_symlink<F: Filesystem + ?Sized>(fs: &F, name: &str, target: &[u8]) {
    let link = lookup(fs, NodeId::ROOT, name);
    assert_eq!(
        link.attr.kind,
        NodeKind::Symlink,
        "{name} is reported as a symlink"
    );
    let mut buf = b"junk".to_vec();
    fs.readlink(link.id, &mut buf)
        .expect("readlink on a symlink");
    assert_eq!(buf, target, "readlink replaces the buffer with the target");
    let through = fs.lookup(link.id, Name::try_from("a.txt").expect("valid name"));
    assert_eq!(
        through,
        Err(Errno::NotDir),
        "lookup never follows a symlink"
    );
}

fn lookup<F: Filesystem + ?Sized>(fs: &F, parent: NodeId, name: &str) -> Entry {
    let name = Name::try_from(name).expect("valid fixture name");
    fs.lookup(parent, name)
        .unwrap_or_else(|e| panic!("lookup {name:?}: {e}"))
}

fn read_all<F: Filesystem + ?Sized>(fs: &F, id: NodeId, fh: Handle, chunk: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0; chunk];
    loop {
        let n = fs.read(id, fh, out.len() as u64, &mut buf).expect("read");
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn root_is_a_directory<F: Filesystem + ?Sized>(fs: &F) {
    let root = fs.getattr(NodeId::ROOT).expect("getattr root");
    assert_eq!(root.kind, NodeKind::Dir, "root is a directory");
}

fn lookup_resolves_kinds_and_sizes<F: Filesystem + ?Sized>(fs: &F) {
    let hello = lookup(fs, NodeId::ROOT, "hello.txt");
    assert_eq!(hello.attr.kind, NodeKind::File);
    assert_eq!(hello.attr.size, HELLO.len() as u64);
    assert_ne!(hello.id, NodeId::ROOT);

    let dir = lookup(fs, NodeId::ROOT, "dir");
    assert_eq!(dir.attr.kind, NodeKind::Dir);
    let big = lookup(fs, dir.id, "big.bin");
    assert_eq!(big.attr.size, BIG_LEN as u64);
    assert_eq!(lookup(fs, dir.id, "empty").attr.kind, NodeKind::Dir);
    assert_eq!(
        fs.getattr(big.id).expect("getattr big.bin").size,
        BIG_LEN as u64
    );
}

fn lookup_failures<F: Filesystem + ?Sized>(fs: &F) {
    let missing = Name::try_from("missing").expect("valid name");
    assert_eq!(
        fs.lookup(NodeId::ROOT, missing),
        Err(Errno::NoEnt),
        "missing name"
    );
    let hello = lookup(fs, NodeId::ROOT, "hello.txt");
    assert_eq!(
        fs.lookup(hello.id, missing),
        Err(Errno::NotDir),
        "lookup inside a file"
    );
    let unknown = NodeId::new(u64::MAX).expect("non-zero");
    assert_eq!(
        fs.getattr(unknown),
        Err(Errno::Stale),
        "getattr of an unknown id"
    );
    assert_eq!(
        fs.lookup(unknown, missing),
        Err(Errno::Stale),
        "lookup under an unknown id"
    );
}

fn ids_are_stable<F: Filesystem + ?Sized>(fs: &F) {
    let dir = lookup(fs, NodeId::ROOT, "dir");
    let first = lookup(fs, dir.id, "a.txt");
    let second = lookup(fs, dir.id, "a.txt");
    assert_eq!(first.id, second.id, "the same name resolves to the same id");
    let other = lookup(fs, NodeId::ROOT, "hello.txt");
    assert_ne!(first.id, other.id, "different nodes have different ids");
    let mut sink = CappedSink::new(usize::MAX);
    fs.readdir(dir.id, Cookie::START, Verifier::NONE, &mut sink)
        .expect("readdir dir");
    let listed = sink
        .entries
        .iter()
        .find(|(name, ..)| name == b"a.txt")
        .expect("a.txt listed");
    assert_eq!(listed.1, first.id, "readdir and lookup agree on ids");
    fs.forget(first.id, 2);
}

fn reads_whole_ranged_and_past_end<F: Filesystem + ?Sized>(fs: &F) {
    let hello = lookup(fs, NodeId::ROOT, "hello.txt");
    assert_eq!(read_all(fs, hello.id, Handle::NONE, 4096), HELLO);

    let mut buf = [0u8; 5];
    let n = fs
        .read(hello.id, Handle::NONE, 7, &mut buf)
        .expect("ranged read");
    assert_eq!(&buf[..n], &HELLO[7..12], "read at an offset");
    let n = fs
        .read(hello.id, Handle::NONE, HELLO.len() as u64, &mut buf)
        .expect("read at end");
    assert_eq!(n, 0, "read at end of file");
    let n = fs
        .read(hello.id, Handle::NONE, u64::MAX, &mut buf)
        .expect("read far past end");
    assert_eq!(n, 0, "read far past end of file");
    let n = fs
        .read(hello.id, Handle::NONE, 0, &mut [])
        .expect("empty buffer");
    assert_eq!(n, 0, "read into an empty buffer");

    let dir = lookup(fs, NodeId::ROOT, "dir");
    let big = lookup(fs, dir.id, "big.bin");
    assert_eq!(
        read_all(fs, big.id, Handle::NONE, 4096),
        big_contents(),
        "chunked read"
    );
    let mut whole = vec![0u8; BIG_LEN + 10];
    let n = fs
        .read(big.id, Handle::NONE, 0, &mut whole)
        .expect("oversized read");
    assert_eq!(n, BIG_LEN, "a large buffer is filled to end of file");
}

fn reads_through_an_open_handle<F: Filesystem + ?Sized>(fs: &F) {
    let hello = lookup(fs, NodeId::ROOT, "hello.txt");
    let fh = fs
        .open(hello.id, OpenFlags::READ)
        .expect("open for reading");
    assert_eq!(
        read_all(fs, hello.id, fh, 3),
        HELLO,
        "small reads through a handle"
    );
    fs.release(hello.id, fh);
}

fn readdir_lists_each_directory<F: Filesystem + ?Sized>(fs: &F) {
    let set = |names: Vec<Vec<u8>>| names.into_iter().collect::<BTreeSet<_>>();
    let (root, _) = collect_dir(fs, NodeId::ROOT);
    assert_eq!(
        set(root),
        set(vec![b"hello.txt".to_vec(), b"dir".to_vec()]),
        "root listing"
    );
    let dir = lookup(fs, NodeId::ROOT, "dir");
    let (names, _) = collect_dir(fs, dir.id);
    let expected = vec![b"a.txt".to_vec(), b"big.bin".to_vec(), b"empty".to_vec()];
    assert_eq!(set(names), set(expected), "/dir listing");
    let empty = lookup(fs, dir.id, "empty");
    assert!(
        collect_dir(fs, empty.id).0.is_empty(),
        "empty directory lists nothing"
    );
}

fn readdir_resumes_from_every_cookie<F: Filesystem + ?Sized>(fs: &F) {
    let dir = lookup(fs, NodeId::ROOT, "dir");
    let (all, _) = collect_dir(fs, dir.id);
    let mut seen = Vec::new();
    let (mut cookie, mut verifier) = (Cookie::START, Verifier::NONE);
    loop {
        let mut sink = CappedSink::new(1);
        verifier = fs
            .readdir(dir.id, cookie, verifier, &mut sink)
            .expect("paged readdir");
        let Some((name, .., next)) = sink.entries.pop() else {
            break;
        };
        seen.push(name);
        cookie = next;
        assert!(seen.len() <= all.len(), "paging terminates");
    }
    assert_eq!(
        seen, all,
        "one-entry pages yield the same listing in the same order"
    );
    let mut sink = CappedSink::new(0);
    fs.readdir(dir.id, Cookie::START, Verifier::NONE, &mut sink)
        .expect("full sink is not an error");
}

fn readdir_and_read_reject_the_wrong_kind<F: Filesystem + ?Sized>(fs: &F) {
    let hello = lookup(fs, NodeId::ROOT, "hello.txt");
    let mut sink = CappedSink::new(8);
    let listed = fs.readdir(hello.id, Cookie::START, Verifier::NONE, &mut sink);
    assert_eq!(listed, Err(Errno::NotDir), "readdir on a file");
    let mut buf = Vec::new();
    assert_eq!(
        fs.readlink(hello.id, &mut buf),
        Err(Errno::Inval),
        "readlink on a file"
    );
    let dir = lookup(fs, NodeId::ROOT, "dir");
    let read = fs.read(dir.id, Handle::NONE, 0, &mut [0u8; 4]);
    assert_eq!(read, Err(Errno::IsDir), "read on a directory");
}
