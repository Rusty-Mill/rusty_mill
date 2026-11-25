//! Mirrors a real temporary directory (plain NTFS on Windows) and runs the
//! shared conformance assertions plus passthrough-specific checks.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rusty_fuse::{Cookie, Errno, Filesystem, Handle, Name, NodeId, NodeKind, OpenFlags};
use rusty_fuse_conformance::{assert_fixture, collect_dir, fixture, CappedSink, FixtureItem};
use rusty_fuse_passthrough::PassthroughFs;

/// A uniquely named directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "rusty_fuse_passthrough-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.0) {
            eprintln!("could not remove {}: {e}", self.0.display());
        }
    }
}

/// A temp dir holding the conformance fixture under `tree/`, so tests can
/// also place things outside the mirrored root.
fn fixture_dir() -> (TempDir, PathBuf) {
    let tmp = TempDir::new();
    let root = tmp.path().join("tree");
    fs::create_dir(&root).expect("create tree");
    for item in fixture() {
        match item {
            FixtureItem::Dir(rel) => fs::create_dir(root.join(rel)).expect("create fixture dir"),
            FixtureItem::File(rel, data) => fs::write(root.join(rel), data).expect("write file"),
        }
    }
    (tmp, root)
}

fn name(s: &str) -> Name<'_> {
    Name::try_from(s).expect("valid test name")
}

#[test]
fn passthrough_passes_the_shared_assertions() {
    let (_tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror the fixture");
    assert_fixture(&fs);
}

#[test]
fn root_must_be_an_existing_directory() {
    let (tmp, root) = fixture_dir();
    assert_eq!(
        PassthroughFs::new(tmp.path().join("absent")).err(),
        Some(Errno::NoEnt)
    );
    assert_eq!(
        PassthroughFs::new(root.join("hello.txt")).err(),
        Some(Errno::NotDir)
    );
}

#[test]
fn opening_for_write_is_refused() {
    let (_tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror");
    let hello = fs.lookup(NodeId::ROOT, name("hello.txt")).expect("lookup");
    assert_eq!(fs.open(hello.id, OpenFlags::WRITE), Err(Errno::ReadOnly));
}

#[test]
fn handles_belong_to_their_node_and_die_on_release() {
    let (_tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror");
    let hello = fs
        .lookup(NodeId::ROOT, name("hello.txt"))
        .expect("lookup hello");
    let dir = fs.lookup(NodeId::ROOT, name("dir")).expect("lookup dir");
    let a = fs.lookup(dir.id, name("a.txt")).expect("lookup a");
    let fh = fs.open(hello.id, OpenFlags::READ).expect("open");
    let mut buf = [0u8; 4];
    assert_eq!(
        fs.read(a.id, fh, 0, &mut buf),
        Err(Errno::BadHandle),
        "handle of another node"
    );
    fs.release(hello.id, fh);
    assert_eq!(
        fs.read(hello.id, fh, 0, &mut buf),
        Err(Errno::BadHandle),
        "released handle"
    );
    assert_eq!(
        fs.read(hello.id, Handle(9_999), 0, &mut buf),
        Err(Errno::BadHandle)
    );
}

#[test]
fn host_changes_show_through_and_invalidate_cookies() {
    let (_tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror");
    let (before, verifier) = collect_dir(&fs, NodeId::ROOT);
    fs::write(root.join("new.txt"), b"fresh").expect("add a file on the host");
    let mut sink = CappedSink::new(8);
    let resumed = fs.readdir(NodeId::ROOT, Cookie(1), verifier, &mut sink);
    assert_eq!(
        resumed,
        Err(Errno::Stale),
        "old cookie after the directory changed"
    );
    let (after, fresh) = collect_dir(&fs, NodeId::ROOT);
    assert_ne!(fresh, verifier);
    assert_eq!(after.len(), before.len() + 1);
    let new = fs
        .lookup(NodeId::ROOT, name("new.txt"))
        .expect("new file is visible");
    assert_eq!(new.attr.size, 5);
}

#[test]
fn deleted_nodes_report_noent() {
    let (_tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror");
    let hello = fs.lookup(NodeId::ROOT, name("hello.txt")).expect("lookup");
    fs::remove_file(root.join("hello.txt")).expect("delete on the host");
    assert_eq!(fs.getattr(hello.id), Err(Errno::NoEnt));
    assert_eq!(
        fs.lookup(NodeId::ROOT, name("hello.txt")),
        Err(Errno::NoEnt)
    );
}

#[test]
fn forget_drops_the_node_and_ids_are_not_reused() {
    let (_tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror");
    let first = fs.lookup(NodeId::ROOT, name("hello.txt")).expect("lookup");
    fs.forget(first.id, 1);
    assert_eq!(
        fs.getattr(first.id),
        Err(Errno::Stale),
        "forgotten id is unknown"
    );
    let again = fs
        .lookup(NodeId::ROOT, name("hello.txt"))
        .expect("lookup again");
    assert_ne!(again.id, first.id);
    fs.forget(NodeId::ROOT, u64::MAX);
    assert!(fs.getattr(NodeId::ROOT).is_ok(), "root is never forgotten");
}

#[test]
fn names_that_could_leave_the_root_are_refused() {
    let (_tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror");
    assert_eq!(
        Name::new(b".."),
        Err(Errno::Inval),
        "the port refuses .. itself"
    );
    assert_eq!(Name::new(b"../x"), Err(Errno::Inval), "and / separators");
    if cfg!(windows) {
        for bad in ["..\\..\\x", "dir\\a.txt", "C:", "hello.txt:stream"] {
            assert_eq!(
                fs.lookup(NodeId::ROOT, name(bad)),
                Err(Errno::Inval),
                "{bad}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn symlinks_are_served_and_never_followed() {
    use rusty_fuse::Verifier;
    use std::os::unix::fs::symlink;
    let (tmp, root) = fixture_dir();
    let outside = tmp.path().join("outside");
    fs::create_dir(&outside).expect("create outside dir");
    fs::write(outside.join("secret.txt"), b"secret").expect("write secret");
    symlink(&outside, root.join("escape")).expect("dir symlink");
    symlink("dir/a.txt", root.join("link")).expect("file symlink");

    let fs = PassthroughFs::new(&root).expect("mirror");
    rusty_fuse_conformance::assert_symlink(&fs, "link", b"dir/a.txt");
    let escape = fs
        .lookup(NodeId::ROOT, name("escape"))
        .expect("lookup escape");
    assert_eq!(escape.attr.kind, NodeKind::Symlink);
    assert_eq!(fs.lookup(escape.id, name("secret.txt")), Err(Errno::NotDir));
    let mut sink = CappedSink::new(8);
    let listed = fs.readdir(escape.id, Cookie::START, Verifier::NONE, &mut sink);
    assert_eq!(
        listed,
        Err(Errno::NotDir),
        "readdir does not follow a directory symlink"
    );
    let link = fs.lookup(NodeId::ROOT, name("link")).expect("lookup link");
    assert_eq!(
        fs.read(link.id, Handle::NONE, 0, &mut [0u8; 8]),
        Err(Errno::Inval)
    );
}

#[cfg(unix)]
#[test]
fn an_ancestor_swapped_for_a_symlink_is_refused() {
    use std::os::unix::fs::symlink;
    let (tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror");
    let dir = fs.lookup(NodeId::ROOT, name("dir")).expect("lookup dir");
    let a = fs.lookup(dir.id, name("a.txt")).expect("lookup a.txt");

    let outside = tmp.path().join("outside");
    fs::create_dir(&outside).expect("create outside dir");
    fs::write(outside.join("a.txt"), b"secret").expect("write decoy");
    fs::rename(root.join("dir"), tmp.path().join("moved")).expect("move dir away");
    symlink(&outside, root.join("dir")).expect("swap in a symlink");

    assert_eq!(
        fs.read(a.id, Handle::NONE, 0, &mut [0u8; 8]),
        Err(Errno::NotDir)
    );
    assert_eq!(fs.lookup(dir.id, name("a.txt")), Err(Errno::NotDir));
}

#[cfg(windows)]
#[test]
fn windows_lookups_follow_ntfs_case_rules() {
    let (_tmp, root) = fixture_dir();
    let fs = PassthroughFs::new(&root).expect("mirror");
    let upper = fs
        .lookup(NodeId::ROOT, name("HELLO.TXT"))
        .expect("NTFS is case-insensitive");
    assert_eq!(upper.attr.kind, NodeKind::File);
}
