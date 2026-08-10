//! Parity suite, Windows leg (RFC v2 §9): the same behavior-spec-derived
//! assertion set as `platform-linux/tests/parity.rs`, run against the
//! Windows backend and the mock. Kept textually identical to the Linux
//! copy's `assert_fs_behavior` on purpose — extraction into a shared crate
//! is the recorded follow-up once a third backend would otherwise mean a
//! third copy (see `docs/behavior/fs.md`).

#![cfg(windows)]
// `windows_metadata_reports_a_real_nlink_and_mtime` makes one raw
// `windows_sys` call directly (bypassing this crate's own code
// entirely) to verify `Metadata::nlink` against a genuinely separate
// path, the same reasoning `net_nonblocking.rs` gives for its own raw
// `windows_sys` call.
#![allow(unsafe_code)]

use std::ffi::OsStr;

use platform::error::ErrorKind;
use platform::fs::{AccessMode, Dir, FileType, Mode, OpenOptions};

/// The shared assertions. Grows with `docs/behavior/fs.md`.
fn assert_fs_behavior(root: &dyn Dir) {
    // create → write → read round-trip, byte-faithful
    let mut f = root
        .open(OsStr::new("a.bin"), &OpenOptions::create_truncate())
        .expect("create");
    f.write(b"one \xff two").expect("write");
    drop(f);
    let mut f = root
        .open(OsStr::new("a.bin"), &OpenOptions::read())
        .expect("open");
    let mut buf = [0u8; 64];
    let n = f.read(&mut buf).expect("read");
    assert_eq!(&buf[..n], b"one \xff two");
    // Drop this read handle now: "a.bin" gets renamed forward to b.bin,
    // then c.bin, then d.bin later in this function (rename just relinks
    // the name, same underlying file), and on Windows a lingering open
    // handle keeps whatever name the file currently has visible in
    // directory enumeration even after remove_file marks it for
    // deletion. Shadowing `f` below does not drop this binding early —
    // only an explicit drop, or end of scope, does.
    drop(f);

    // metadata: type and length agree with what was written
    let md = root.metadata(OsStr::new("a.bin")).expect("metadata");
    assert_eq!(md.file_type, FileType::File);
    assert_eq!(md.len, 9);
    // Portable across backends (coreutils gap backlog #63): a real
    // link count is always at least 1, and a file just written can't
    // report a modification time in the future. Backend-specific
    // values (an exact mtime, a >1 nlink for a real hard link) get
    // their own live-verified test —
    // `windows_metadata_reports_a_real_nlink_and_mtime`.
    assert!(md.nlink >= 1, "nlink must be at least 1");
    assert!(
        md.modified <= std::time::SystemTime::now(),
        "a freshly-written file can't be modified in the future"
    );

    // missing entries are NotFound with path context
    let e = root.metadata(OsStr::new("missing")).expect_err("must fail");
    assert_eq!(e.kind, ErrorKind::NotFound);
    assert!(e.path.is_some());

    // create_new refuses to clobber
    let e = root
        .open(
            OsStr::new("a.bin"),
            &OpenOptions {
                write: true,
                create_new: true,
                ..Default::default()
            },
        )
        .err()
        .expect("create_new over existing must fail");
    assert_eq!(e.kind, ErrorKind::AlreadyExists);

    // directories: create, list through a child capability, refuse
    // non-empty removal, then remove bottom-up
    root.create_dir(OsStr::new("d")).expect("mkdir");
    let d = root.open_dir(OsStr::new("d")).expect("open_dir");
    d.open(OsStr::new("inner"), &OpenOptions::create_truncate())
        .expect("create");
    let names: Vec<_> = d
        .read_dir()
        .expect("read_dir")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.contains(&OsStr::new("inner").to_os_string()));
    let e = root
        .remove_dir(OsStr::new("d"))
        .expect_err("non-empty removal must fail");
    assert_eq!(e.kind, ErrorKind::DirectoryNotEmpty);
    d.remove_file(OsStr::new("inner")).expect("rm inner");
    // Drop the capability before removing the directory it names: on
    // Windows, delete-on-close semantics mean a directory entry stays
    // visible in enumeration until every open handle to it — including
    // this one — closes, not just the handle `remove_dir` itself opens
    // and releases. Linux would tolerate the leak (unlink/rmdir detach
    // the name immediately regardless of open fds), but the capability
    // model says: done with it, drop it.
    drop(d);
    root.remove_dir(OsStr::new("d")).expect("rmdir");

    // rename: replaces by default, no-replace refuses (D11, roadmap
    // Phase 3). "a.bin" (still 9 bytes from the round-trip above) moves
    // to "b.bin"; the old name is gone, the new one reads back intact.
    root.rename(OsStr::new("a.bin"), OsStr::new("b.bin"))
        .expect("rename");
    assert_eq!(
        root.metadata(OsStr::new("a.bin")).unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert_eq!(root.metadata(OsStr::new("b.bin")).unwrap().len, 9);

    // rename replaces an existing destination by default...
    root.open(OsStr::new("c.bin"), &OpenOptions::create_truncate())
        .expect("create c");
    root.rename(OsStr::new("b.bin"), OsStr::new("c.bin"))
        .expect("rename replaces c.bin");
    assert_eq!(root.metadata(OsStr::new("c.bin")).unwrap().len, 9);
    assert_eq!(
        root.metadata(OsStr::new("b.bin")).unwrap_err().kind,
        ErrorKind::NotFound
    );

    // ...but rename_no_replace refuses when the destination is a
    // DIFFERENT existing entry, atomically (no partial move: "c.bin"
    // and "e.bin" are both untouched). Deliberately not "rename c.bin
    // onto its own name" — that degenerate case is a real, harmless
    // cross-OS divergence (Linux's renameat2(RENAME_NOREPLACE) refuses
    // it with EEXIST; Windows' NtSetInformationFile treats renaming a
    // file onto its own current name as a no-op success), not the
    // atomic-refuse-if-exists contract this assertion means to pin.
    root.open(OsStr::new("e.bin"), &OpenOptions::create_truncate())
        .expect("create e");
    let e = root
        .rename_no_replace(OsStr::new("c.bin"), OsStr::new("e.bin"))
        .expect_err("must refuse: e.bin already exists");
    assert_eq!(e.kind, ErrorKind::AlreadyExists);
    assert_eq!(root.metadata(OsStr::new("c.bin")).unwrap().len, 9);
    assert_eq!(root.metadata(OsStr::new("e.bin")).unwrap().len, 0);
    root.remove_file(OsStr::new("e.bin")).expect("rm e.bin");

    root.rename_no_replace(OsStr::new("c.bin"), OsStr::new("d.bin"))
        .expect("no existing destination: must succeed");
    root.remove_file(OsStr::new("d.bin")).expect("rm d.bin");

    // write_atomic: publishes contents under `rel`, leaves no temp file
    // behind, and a second call (replace) fully overwrites the first.
    root.write_atomic(OsStr::new("atomic.txt"), b"first")
        .expect("write_atomic 1");
    let mut f = root
        .open(OsStr::new("atomic.txt"), &OpenOptions::read())
        .expect("open atomic");
    let mut buf = [0u8; 64];
    let n = f.read(&mut buf).expect("read atomic");
    assert_eq!(&buf[..n], b"first");
    drop(f);

    root.write_atomic(OsStr::new("atomic.txt"), b"second, shorter overwrite")
        .expect("write_atomic 2");
    let mut f = root
        .open(OsStr::new("atomic.txt"), &OpenOptions::read())
        .expect("re-open atomic");
    let n = f.read(&mut buf).expect("re-read atomic");
    assert_eq!(&buf[..n], b"second, shorter overwrite");
    drop(f);

    // symlink/read_link (symlink slice): the target is stored verbatim,
    // not validated or resolved — a dangling target is fine to create.
    // metadata (lstat-style, never follows) classifies the link itself
    // as Symlink; read_link round-trips the exact bytes given.
    match root.symlink(OsStr::new("nowhere/dangling"), OsStr::new("link1")) {
        Ok(()) => {
            assert_eq!(
                root.metadata(OsStr::new("link1")).unwrap().file_type,
                FileType::Symlink
            );
            assert_eq!(
                root.read_link(OsStr::new("link1")).unwrap(),
                OsStr::new("nowhere/dangling").to_os_string()
            );
            // Like `open` with `create_new`, `symlink` refuses to clobber an
            // existing name rather than silently replacing it.
            let e = root
                .symlink(OsStr::new("whatever"), OsStr::new("link1"))
                .expect_err("must refuse: link1 already exists");
            assert_eq!(e.kind, ErrorKind::AlreadyExists);
            // `read_link` on a non-symlink refuses, mirroring POSIX `EINVAL`.
            let e = root
                .read_link(OsStr::new("atomic.txt"))
                .expect_err("not a symlink");
            assert_eq!(e.kind, ErrorKind::InvalidInput);
            root.remove_file(OsStr::new("link1")).expect("rm link1");

            // A symlink to an existing directory
            root.create_dir(OsStr::new("realdir"))
                .expect("mkdir realdir");
            if let Ok(()) = root.symlink(OsStr::new("realdir"), OsStr::new("dirlink")) {
                assert_eq!(
                    root.metadata(OsStr::new("dirlink")).unwrap().file_type,
                    FileType::Symlink
                );
                assert_eq!(
                    root.read_link(OsStr::new("dirlink")).unwrap(),
                    OsStr::new("realdir").to_os_string()
                );
                if root.remove_file(OsStr::new("dirlink")).is_err() {
                    root.remove_dir(OsStr::new("dirlink")).expect("rm dirlink");
                }
            }
            root.remove_dir(OsStr::new("realdir")).expect("rm realdir");
        }
        Err(e) if e.to_string().contains("1314") || e.to_string().contains("privilege") => {
            let _ = root.remove_file(OsStr::new("link1"));
            println!("Skipping symlink test: SeCreateSymbolicLinkPrivilege (Win32 1314) not held by current user process");
        }
        Err(e) => panic!("symlink failed with unexpected error: {:?}", e),
    }

    // access (faccessat slice): read/write on a file we own, execute
    // (search) on a directory we own — the one case execute permission
    // is genuinely testable identically on both backends. A regular
    // file's execute bit is the Windows divergence
    // (docs/divergences.md #005): pinned by dedicated backend-only
    // tests, not asserted here.
    root.access(OsStr::new("atomic.txt"), AccessMode::read())
        .expect("can read what we own");
    root.access(
        OsStr::new("atomic.txt"),
        AccessMode {
            read: true,
            write: true,
            execute: false,
        },
    )
    .expect("can read+write what we own");
    root.create_dir(OsStr::new("accessdir"))
        .expect("mkdir accessdir");
    root.access(OsStr::new("accessdir"), AccessMode::execute())
        .expect("can search a directory we own");
    root.remove_dir(OsStr::new("accessdir"))
        .expect("rm accessdir");
    let e = root
        .access(OsStr::new("missing"), AccessMode::read())
        .expect_err("must fail: missing");
    assert_eq!(e.kind, ErrorKind::NotFound);
    // An empty mode is a vacuous yes, even for a name that doesn't
    // exist — existence is metadata's job, not this one's.
    root.access(OsStr::new("also-missing"), AccessMode::default())
        .expect("empty mode never fails, even for a name that doesn't exist");

    // unix_mode/file_id (test predicates' donor material, D11's
    // faccessat-slice sibling): unix_mode is a real answer on Linux/mock,
    // `None` on Windows (no such concept) — both are valid per the
    // contract, so only check contents when `Some`. file_id is
    // answerable on every backend: the same entry queried twice yields
    // equal ids, and two distinct entries yield different ones.
    if let Some(um) = root.unix_mode(OsStr::new("atomic.txt")).unwrap() {
        assert!(!um.setuid);
        assert!(!um.setgid);
        assert!(!um.sticky);
    }
    let id_a = root.file_id(OsStr::new("atomic.txt")).unwrap();
    let id_a_again = root.file_id(OsStr::new("atomic.txt")).unwrap();
    assert_eq!(id_a, id_a_again, "same entry, same id");
    root.create_dir(OsStr::new("otherdir"))
        .expect("mkdir otherdir");
    let id_b = root.file_id(OsStr::new("otherdir")).unwrap();
    assert_ne!(id_a, id_b, "different entries, different ids");
    root.remove_dir(OsStr::new("otherdir"))
        .expect("rm otherdir");

    let names: Vec<_> = root
        .read_dir()
        .expect("read_dir")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(
        names,
        vec![OsStr::new("atomic.txt").to_os_string()],
        "no leftover temp file, no leftover a/b/c.bin"
    );
    root.remove_file(OsStr::new("atomic.txt")).expect("rm");
}

#[test]
fn mock_backend_conforms() {
    assert_fs_behavior(&platform_mock::MockDir::root());
}

#[test]
fn windows_backend_conforms() {
    let tmp = std::env::temp_dir().join(format!("rustils-parity-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");
    assert_fs_behavior(&root);
    std::fs::remove_dir_all(&tmp).ok();
}

/// Divergence #005 (docs/divergences.md), Windows side: a plain data
/// file has no execute-permission bit for `access` to check at all
/// (execute is a property of file type/extension, not an ACL bit) — it
/// is granted unconditionally once existence is confirmed. Linux's own
/// pinning test (`linux_access_denies_execute_on_a_plain_file`) asserts
/// the opposite for the identical setup.
#[test]
fn windows_access_grants_execute_unconditionally() {
    let tmp = std::env::temp_dir().join(format!("rustils-access-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");
    root.open(OsStr::new("f"), &OpenOptions::create_truncate())
        .expect("create f");
    root.access(OsStr::new("f"), AccessMode::execute())
        .expect("Windows has no execute bit to deny on a plain data file");
    std::fs::remove_dir_all(&tmp).ok();
}

/// `Metadata::nlink`/`modified` (coreutils gap backlog #63, `ls -l`
/// donor material). `modified` is checked against
/// `std::fs::Metadata::modified()` (std's own independent Windows
/// metadata path); `nlink` has no *stable* std accessor
/// (`MetadataExt::number_of_links` is nightly-only,
/// `windows_by_handle` — rust-lang/rust#63010), so it's checked
/// against a raw `GetFileInformationByHandleEx(FileStandardInfo, ...)`
/// call issued directly by the test instead, bypassing this crate's
/// own `windows_sys` calls entirely — the same "verify against a
/// genuinely separate code path" discipline
/// `linux_metadata_reports_a_real_nlink_mtime_and_permissions` uses
/// via a raw `libc::stat` call.
#[test]
fn windows_metadata_reports_a_real_nlink_and_mtime() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{FileStandardInfo, GetFileInformationByHandleEx};

    let tmp = std::env::temp_dir().join(format!("rustils-metadata-ext-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");
    root.open(OsStr::new("f"), &OpenOptions::create_truncate())
        .expect("create f");

    let md = root.metadata(OsStr::new("f")).expect("metadata");
    let std_md = std::fs::metadata(tmp.join("f")).expect("std::fs::metadata");
    assert_eq!(
        md.modified,
        std_md.modified().expect("std modified()"),
        "mtime must match std's own independent Windows metadata"
    );

    #[repr(C)]
    struct FileStandardInfoLayout {
        allocation_size: i64,
        end_of_file: i64,
        number_of_links: u32,
        delete_pending: u8,
        directory: u8,
    }
    let std_file = std::fs::File::open(tmp.join("f")).expect("std open for raw handle");
    // SAFETY: `std_file`'s raw handle is open and alive for the
    // duration of this call; `info` is a valid out-buffer of exactly
    // the queried class's size, outliving the call.
    let info: FileStandardInfoLayout = unsafe {
        let mut info = std::mem::zeroed::<FileStandardInfoLayout>();
        let ok = GetFileInformationByHandleEx(
            std_file.as_raw_handle() as _,
            FileStandardInfo,
            (&mut info as *mut FileStandardInfoLayout).cast(),
            std::mem::size_of::<FileStandardInfoLayout>() as u32,
        );
        assert_ne!(ok, 0, "raw GetFileInformationByHandleEx must succeed");
        info
    };
    assert_eq!(
        md.nlink,
        u64::from(info.number_of_links),
        "nlink must match a raw GetFileInformationByHandleEx(FileStandardInfo) call"
    );

    drop(std_file);
    std::fs::remove_dir_all(&tmp).ok();
}

/// `unix_mode` has no Windows analog at all (NTFS security descriptors,
/// not POSIX mode bits/uid/gid) — pinning the honest `None` answer, not
/// a fabricated zeroed-out `Some`.
#[test]
fn windows_unix_mode_is_always_none() {
    let tmp = std::env::temp_dir().join(format!("rustils-unixmode-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");
    root.open(OsStr::new("f"), &OpenOptions::create_truncate())
        .expect("create f");
    assert_eq!(root.unix_mode(OsStr::new("f")).unwrap(), None);
    std::fs::remove_dir_all(&tmp).ok();
}

/// `set_unix_mode` (coreutils gap backlog #64) has the same "no analog"
/// gap as `unix_mode` above, but on the write side (divergence 009): a
/// silent no-op would misrepresent success, so this is `Unsupported`,
/// not `Ok(())`.
#[test]
fn windows_set_unix_mode_is_unsupported() {
    let tmp = std::env::temp_dir().join(format!("rustils-setunixmode-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");
    root.open(OsStr::new("f"), &OpenOptions::create_truncate())
        .expect("create f");
    let e = root
        .set_unix_mode(OsStr::new("f"), Mode::default())
        .expect_err("no POSIX mode-bit concept on Windows");
    assert_eq!(e.kind, ErrorKind::Unsupported);
    std::fs::remove_dir_all(&tmp).ok();
}

/// Process behavior (docs/behavior/process.md) against the native
/// backend. Fixtures are OS-specific (`cmd`); the assertions mirror the
/// Linux leg's. `Signaled` has no Windows fixture — the spec pins that it
/// is never produced here.
#[test]
fn windows_process_backend_conforms() {
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    use platform::process::{Command, EnvSpec, ExitStatus, Spawner, Stdio};

    let tmp = std::env::temp_dir().join(format!("rustils-proc-parity-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let s = platform_windows::WindowsSpawner;

    // Exit codes decode uniformly. `cmd` resolves via PATH + PATHEXT.
    let c = Command::new("cmd", tmp.clone()).arg("/c").arg("exit 7");
    let child = s.spawn(&c).expect("spawn");
    assert!(child.id() > 0);
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(7));

    // cwd is honored: the child sees the marker only if it starts there.
    std::fs::write(tmp.join("rustils-marker.txt"), b"x").expect("marker");
    let c = Command::new("cmd", tmp.clone())
        .arg("/c")
        .arg("if exist rustils-marker.txt (exit 0) else (exit 1)");
    assert!(s.spawn(&c).expect("spawn").wait().expect("wait").success());

    // Explicit env starts empty: only the given variables are visible.
    // SystemRoot rides along — cmd.exe itself misbehaves without it, and
    // inheriting it says nothing about leakage of arbitrary variables.
    let mut env = BTreeMap::new();
    env.insert(OsString::from("RUSTILS_CODE"), OsString::from("42"));
    if let Some(sr) = std::env::var_os("SystemRoot") {
        env.insert(OsString::from("SystemRoot"), sr);
    }
    let c = Command::new("cmd", tmp.clone())
        .arg("/c")
        .arg("exit %RUSTILS_CODE%")
        .env(EnvSpec::Explicit(env));
    assert_eq!(
        s.spawn(&c).expect("spawn").wait().expect("wait"),
        ExitStatus::Code(42)
    );

    // Stdio::Null wiring spawns and completes.
    let mut c = Command::new("cmd", tmp.clone())
        .arg("/c")
        .arg("echo swallowed");
    c.stdout = Stdio::Null;
    assert!(s.spawn(&c).expect("spawn").wait().expect("wait").success());

    // resolve: mechanism-level NotFound with path context.
    let e = s
        .resolve(std::ffi::OsStr::new("rustils-definitely-missing"))
        .expect_err("must fail");
    assert_eq!(e.kind, platform::error::ErrorKind::NotFound);
    assert!(e.path.is_some());

    std::fs::remove_dir_all(&tmp).ok();
}

/// Groups and kill-tree (divergence 001 pin, Windows side). The sleeper
/// (`ping -n 30` ≈ 29s) completing inside cargo's normal budget IS the
/// kill assertion (wall-clock discipline, extraction map D7); instant-
/// exit stand-ins keep the refusal case race-free.
#[test]
fn windows_process_group_kill() {
    use platform::process::{Command, ExitStatus, GroupSpec, Signal, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;

    let sleeper = |group: GroupSpec| {
        let mut c = Command::new("ping", tmp.clone())
            .arg("-n")
            .arg("30")
            .arg("127.0.0.1")
            .group(group);
        c.stdout = Stdio::Null;
        c
    };

    // kill_tree on a NewGroup child: TerminateJobObject reaches the job;
    // wait reports Code(1) — Windows has no signal to encode (divergence
    // 001, the cross-OS pin opposite Linux's Signaled(9)).
    let child = s.spawn(&sleeper(GroupSpec::NewGroup)).expect("spawn");
    child.kill_tree(Signal::Kill).expect("kill_tree");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(1));

    // kill_tree reaches a grandchild: cmd spawns ping as its own child;
    // killing the job takes both down promptly.
    let mut c = Command::new("cmd", tmp.clone())
        .arg("/c")
        .arg("ping -n 30 127.0.0.1 >NUL")
        .group(GroupSpec::NewGroup);
    c.stdout = Stdio::Null;
    let child = s.spawn(&c).expect("spawn");
    std::thread::sleep(std::time::Duration::from_millis(100));
    child.kill_tree(Signal::Kill).expect("kill_tree");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(1));

    // kill_single works without a group.
    let child = s.spawn(&sleeper(GroupSpec::Inherit)).expect("spawn");
    child.kill_single(Signal::Kill).expect("kill_single");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(1));

    // kill_tree without NewGroup is refused, not guessed at.
    let c = Command::new("cmd", tmp).arg("/c").arg("exit 0");
    let child = s.spawn(&c).expect("spawn");
    assert_eq!(
        child.kill_tree(Signal::Kill).expect_err("must refuse").kind,
        platform::error::ErrorKind::Unsupported
    );
    child.wait().expect("wait");
}

/// try_wait + wait_any (extraction map step 3 seed, Windows side) —
/// mirrors the Linux leg's assertions with cmd/ping fixtures.
#[test]
fn windows_wait_any_and_try_wait() {
    use platform::process::{wait_any, Child, Command, ExitStatus, Signal, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;

    // try_wait: None while running; Some after; wait returns the stash.
    let c = Command::new("cmd", tmp.clone()).arg("/c").arg("exit 4");
    let mut child: Box<dyn Child> = s.spawn(&c).expect("spawn");
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            assert_eq!(status, ExitStatus::Code(4));
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(child.try_wait().expect("repoll"), Some(ExitStatus::Code(4)));
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(4));

    // wait_any: returns the quick child's index, sleeper still running.
    let mut sleeper_cmd = Command::new("ping", tmp.clone())
        .arg("-n")
        .arg("30")
        .arg("127.0.0.1");
    sleeper_cmd.stdout = Stdio::Null;
    let sleeper = s.spawn(&sleeper_cmd).expect("spawn");
    let quick = s
        .spawn(&Command::new("cmd", tmp.clone()).arg("/c").arg("exit 9"))
        .expect("spawn");
    let mut children: Vec<Box<dyn Child>> = vec![sleeper, quick];
    let idx = s
        .wait_any(&mut children, None)
        .expect("wait_any")
        .expect("no timeout");
    assert_eq!(idx, 1, "the quick child finishes first");
    assert_eq!(
        children.remove(1).wait().expect("wait"),
        ExitStatus::Code(9)
    );

    // Timeout: the sleeper alone times out promptly.
    let waited = s
        .wait_any(&mut children, Some(std::time::Duration::from_millis(50)))
        .expect("wait_any");
    assert_eq!(waited, None);

    // Empty set is refused like the OS primitives refuse it.
    let mut none: Vec<Box<dyn Child>> = Vec::new();
    assert_eq!(
        wait_any(&mut none, None).expect_err("must refuse").kind,
        platform::error::ErrorKind::InvalidInput
    );
    assert_eq!(
        s.wait_any(&mut none, None).expect_err("must refuse").kind,
        platform::error::ErrorKind::InvalidInput
    );

    // Release the sleeper.
    children[0].kill_single(Signal::Kill).expect("kill");
    children.remove(0).wait().expect("wait");
}

/// Pipes (extraction map step 4, Windows side): capture with EOF at child
/// exit (BROKEN_PIPE decoded as end-of-file), and stdin feeding with
/// EOF-by-drop through findstr.
#[test]
fn windows_process_pipes() {
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;

    // stdout capture: bytes arrive, then EOF at child exit.
    let mut c = Command::new("cmd", tmp.clone())
        .arg("/c")
        .arg("echo captured");
    c.stdout = Stdio::Pipe;
    let mut child = s.spawn(&c).expect("spawn");
    let mut pipe = child.take_stdout().expect("piped stdout");
    assert!(child.take_stdout().is_none(), "take is once");
    let mut got = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = pipe.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    // cmd's echo appends \r\n.
    assert_eq!(got, b"captured\r\n");
    assert!(child.wait().expect("wait").success());

    // stdin feeding: findstr "^" echoes every stdin line after EOF.
    let mut c = Command::new("findstr", tmp).arg("^");
    c.stdin = Stdio::Pipe;
    c.stdout = Stdio::Pipe;
    let mut child = s.spawn(&c).expect("spawn");
    let mut stdin = child.take_stdin().expect("piped stdin");
    let mut stdout = child.take_stdout().expect("piped stdout");
    stdin.write(b"round trip\r\n").expect("write");
    drop(stdin); // EOF — findstr exits only once stdin closes (D5)
    let mut got = Vec::new();
    loop {
        let n = stdout.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, b"round trip\r\n");
    assert!(child.wait().expect("wait").success());
}

/// Many children through the multiplexer (R3): 70 processes collected to
/// completion — past WaitForMultipleObjects's 64-handle cap, forcing the
/// chunked sweep that absorbs the documented limit (RFC v2 5.6).
#[test]
fn windows_wait_any_many_children() {
    use platform::process::{Child, Command, ExitStatus, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;
    let mut children: Vec<Box<dyn Child>> = (0..70)
        .map(|i| {
            s.spawn(
                &Command::new("cmd", tmp.clone())
                    .arg("/c")
                    .arg(format!("exit {}", i % 8)),
            )
            .expect("spawn")
        })
        .collect();
    let mut statuses = Vec::new();
    while !children.is_empty() {
        let idx = s
            .wait_any(&mut children, None)
            .expect("wait_any")
            .expect("no timeout");
        statuses.push(children.remove(idx).wait().expect("wait"));
    }
    assert_eq!(statuses.len(), 70);
    assert!(statuses
        .iter()
        .all(|st| matches!(st, ExitStatus::Code(c) if *c < 8)));
}

/// Deferred signals (D6, Windows side). Deliberately NOT asserted here:
/// actual Ctrl-C/Ctrl-Break delivery — GitHub's runners execute tests
/// without an interactive console, and GenerateConsoleCtrlEvent
/// addresses console process groups this harness does not control; a
/// false green from a self-signal that never arrives would be worse
/// than the narrow pin below (extraction map D7's "document what is not
/// asserted" discipline). What IS pinned: installation succeeds and the
/// slot starts (and stays) empty without a delivery.
#[test]
fn windows_signal_source_installs() {
    use platform::events::{SignalEvent, SignalSource};

    let signals = platform_windows::WindowsSignalSource;
    signals
        .install(&[SignalEvent::Interrupt, SignalEvent::Terminate])
        .expect("install");
    assert_eq!(signals.take(), None);
}

/// The redirected-streams contract (`docs/behavior/term.md`): same
/// assertions as the Linux leg — false everywhere, Err for size,
/// refuse raw mode, leave_raw an Ok no-op.
#[cfg(windows)]
#[test]
fn windows_terminal_honest_when_redirected() {
    use platform::term::{TermStream, Terminal};
    let mut t = platform_windows::WindowsTerminal::new();
    assert!(!t.is_tty(TermStream::Stdin));
    assert!(!t.is_tty(TermStream::Stdout));
    assert!(!t.is_tty(TermStream::Stderr));
    assert!(t.window_size().is_err(), "no tty: size must be Err");
    assert!(!t.is_raw(), "no tty: is_raw's live probe must say false");
    assert!(t.enter_raw().is_err(), "no tty: raw mode must refuse");
    t.leave_raw().expect("leave without enter is an Ok no-op");
    assert!(t.set_echo(false).is_err(), "no tty: set_echo must refuse");
}

/// Divergence 008 (Windows side): `Signal::Kill` behaves exactly as
/// `kill_tree`/`kill_single` always did; every other identity is
/// `Unsupported` — there is no OS mechanism to deliver an arbitrary
/// signal to a process here.
#[test]
fn windows_kill_signal_is_kill_only() {
    use platform::process::{Command, ExitStatus, Signal, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;

    let mut c = Command::new("ping", tmp)
        .arg("-n")
        .arg("30")
        .arg("127.0.0.1");
    c.stdout = Stdio::Null;
    let child = s.spawn(&c).expect("spawn");
    assert_eq!(
        child
            .kill_single(Signal::Term)
            .expect_err("must refuse")
            .kind,
        platform::error::ErrorKind::Unsupported
    );
    child.kill_single(Signal::Kill).expect("Kill always works");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(1));
}

/// Divergence 008 (Windows side, the `GroupSpec` half): `JoinGroup`
/// targets a numeric pgid Windows has no analog for — Job Objects are
/// handle-based, with no "start already inside group N" primitive.
/// `spawn` refuses up front rather than silently falling back to
/// `Inherit`/`NewGroup`.
#[test]
fn windows_join_group_is_unsupported() {
    use platform::process::{Command, GroupSpec, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;
    let c = Command::new("cmd", tmp)
        .arg("/c")
        .arg("exit 0")
        .group(GroupSpec::JoinGroup(1));
    assert_eq!(
        s.spawn(&c).err().expect("must refuse").kind,
        platform::error::ErrorKind::Unsupported
    );
}

/// `Spawner::adopt` (rustils#47): a real, already-running pid — standing
/// in for one a third-party library spawned (e.g.
/// `portable-pty::Child::process_id()`) — placed into a fresh
/// kill-on-close Job Object after the fact. `kill_tree` on the *adopted*
/// handle must reach the *original* child (proving `AssignProcessToJobObject`
/// actually landed on the real process, not just that `adopt` returned
/// `Ok`), which the original `Child`'s own `wait()` then observes.
#[test]
fn windows_adopt_places_a_real_pid_into_a_new_job() {
    use platform::process::{Command, ExitStatus, Signal, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;

    let mut c = Command::new("ping", tmp)
        .arg("-n")
        .arg("30")
        .arg("127.0.0.1");
    c.stdout = Stdio::Null;
    let child = s.spawn(&c).expect("spawn");
    let pid = child.id();

    let adopted = s.adopt(pid).expect("adopt a real, running pid");
    adopted.kill_tree(Signal::Kill).expect("kill_tree");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(1));
}

/// `Spawner::adopt` of a pid naming no live process — `OpenProcess`
/// itself fails, surfaced as a real `Err`, not a handle that silently
/// fails every later operation.
#[test]
fn windows_adopt_of_a_dead_pid_fails() {
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;

    // A pid that certainly doesn't name a live process: spawn, wait for
    // exit, then reuse its now-dead pid.
    let mut c = Command::new("cmd", tmp).arg("/c").arg("exit 0");
    c.stdout = Stdio::Null;
    let child = s.spawn(&c).expect("spawn");
    let pid = child.id();
    child.wait().expect("wait for it to actually exit");

    let e = s.adopt(pid);
    // Best-effort: pid reuse means this could theoretically race a new
    // process claiming the same pid, but immediately after `wait()`
    // returns that's not realistically going to happen in CI.
    assert!(
        e.is_err(),
        "adopting a dead pid must fail, not silently succeed"
    );
}

/// D8/D10 (Windows side): no job-control stop/continue analog —
/// `wait_job`/`try_wait_job` are `Unsupported` rather than silently
/// degrading to plain `wait`/`try_wait` semantics.
#[test]
fn windows_wait_job_is_unsupported() {
    use platform::process::{Command, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;
    let c = Command::new("cmd", tmp).arg("/c").arg("exit 0");
    let mut child = s.spawn(&c).expect("spawn");
    assert_eq!(
        child.wait_job().expect_err("must refuse").kind,
        platform::error::ErrorKind::Unsupported
    );
    assert_eq!(
        child.try_wait_job().expect_err("must refuse").kind,
        platform::error::ErrorKind::Unsupported
    );
    child.kill_single(platform::process::Signal::Kill).ok();
    let _ = child.wait();
}

/// `Stdio::File` (rustils#51, D5): a stage's stdin/stdout wired to an
/// already-open `File` instead of `Inherit`/`Null`/`Pipe` — the `< file`
/// and `> file` shell-redirect shapes.
#[test]
fn windows_stdio_file_wiring() {
    use platform::fs::{Dir, OpenOptions};
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir().join(format!("rustils-stdio-file-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let dir = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");
    let s = platform_windows::WindowsSpawner;

    // `< file`: stdin wired to an already-open File, read back through a
    // piped stdout (`findstr "^"` echoes every stdin line, the same
    // fixture `windows_process_pipes` uses) to prove the child actually
    // saw the file's bytes.
    let mut in_file = dir
        .open(OsStr::new("in.txt"), &OpenOptions::create_truncate())
        .expect("create in.txt");
    in_file.write(b"hello from file\r\n").expect("write in.txt");
    drop(in_file);
    let in_read = dir
        .open(OsStr::new("in.txt"), &OpenOptions::read())
        .expect("reopen in.txt for read");

    let mut c = Command::new("findstr", tmp.clone()).arg("^");
    c.stdin = Stdio::File(in_read);
    c.stdout = Stdio::Pipe;
    let mut child = s.spawn(&c).expect("spawn");
    let mut out = child.take_stdout().expect("piped stdout");
    let mut got = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = out.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, b"hello from file\r\n");
    assert!(child.wait().expect("wait").success());

    // `> file`: stdout wired to an already-open File, read back directly
    // from the filesystem afterward.
    let out_file = dir
        .open(OsStr::new("out.txt"), &OpenOptions::create_truncate())
        .expect("create out.txt");
    let mut c = Command::new("cmd", tmp.clone())
        .arg("/c")
        .arg("echo to file");
    c.stdout = Stdio::File(out_file);
    let child = s.spawn(&c).expect("spawn");
    assert!(child.wait().expect("wait").success());
    let mut readback = dir
        .open(OsStr::new("out.txt"), &OpenOptions::read())
        .expect("reopen out.txt");
    let mut got = [0u8; 64];
    let n = readback.read(&mut got).expect("read out.txt");
    // cmd's echo appends \r\n.
    assert_eq!(&got[..n], b"to file\r\n");

    std::fs::remove_dir_all(&tmp).ok();
}

/// `File::try_clone` + `Stdio::File` (rustils#51, D5): the `2>&1`/
/// `&> file` shape — stdout and stderr wired to *clones* of the same
/// open file, which must share the file's offset the way a real
/// `DuplicateHandle` pair does, so sequential writes through either
/// handle append rather than clobber each other at position 0.
#[test]
fn windows_stdio_file_try_clone_shares_offset_for_dup_style_redirect() {
    use platform::fs::{Dir, OpenOptions};
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir().join(format!("rustils-stdio-dup-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let dir = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");
    let s = platform_windows::WindowsSpawner;

    let out_file = dir
        .open(OsStr::new("both.txt"), &OpenOptions::create_truncate())
        .expect("create both.txt");
    let err_file = out_file.try_clone().expect("try_clone");

    // rustils#57: `echo`'s own literal argument text and a redirect
    // operator appended directly after it don't tokenize cleanly —
    // `echo err- 1>&2` echoed `err- ` (the fd-digit token gets
    // stripped, but the space before it doesn't), and `echo
    // err-1>&2` echoed `err-1` (with no separating whitespace, the
    // digit is consumed into the preceding word instead of being
    // recognized as an isolated handle number). Grouping the echo in
    // parens sidesteps both: `echo err-` finishes producing its own
    // output (just `err-\r\n`, nothing appended) before the group
    // closes, so the redirect that follows the closing paren applies
    // to the *group's* stdout — a separate parsing context from
    // echo's own literal-text consumption — with no ambiguity left
    // for a stray digit or space to leak into what was echoed.
    let mut c = Command::new("cmd", tmp.clone())
        .arg("/c")
        .arg("echo out-&(echo err-) 1>&2");
    c.stdout = Stdio::File(out_file);
    c.stderr = Stdio::File(err_file);
    let child = s.spawn(&c).expect("spawn");
    assert!(child.wait().expect("wait").success());

    let mut readback = dir
        .open(OsStr::new("both.txt"), &OpenOptions::read())
        .expect("reopen both.txt");
    let mut got = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = readback.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(
        got, b"out-\r\nerr-\r\n",
        "shared offset: sequential writes append, not clobber"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

/// `Stdio::File` refuses a `File` from a foreign backend (rustils#51):
/// `WindowsSpawner` can only extract a handle from its own `WindowsFile`,
/// so a `platform-mock`-backed `File` fails `Unsupported` rather than
/// the spawn silently ignoring the redirect or panicking.
#[test]
fn windows_stdio_file_refuses_a_foreign_backend_file() {
    use platform::fs::{Dir, OpenOptions};
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_windows::WindowsSpawner;
    let foreign = platform_mock::MockDir::root()
        .with_file("f", b"x".to_vec())
        .open(OsStr::new("f"), &OpenOptions::read())
        .expect("mock open");

    let mut c = Command::new("cmd", tmp).arg("/c").arg("exit 0");
    c.stdin = Stdio::File(foreign);
    let e = s
        .spawn(&c)
        .err()
        .expect("must refuse: foreign File backend");
    assert_eq!(e.kind, platform::error::ErrorKind::Unsupported);
}

/// R2-equivalent containment (`docs/behavior/fs.md`, `docs/
/// divergences.md` #013): `open_dir` now rejects a reparse point sitting
/// in an intermediate path component instead of silently following it —
/// `sys::nt::open_relative_r2`'s `OBJ_DONT_REPARSE`, previously
/// unrequested (plain `open_relative` follows every reparse point
/// transparently, like the Linux R1 baseline it mirrors). Skips honestly
/// (matching `assert_fs_behavior`'s own symlink block) when the test
/// process lacks `SeCreateSymbolicLinkPrivilege` — nothing to prove
/// about reparse-point rejection if no reparse point can be created at
/// all in this environment.
#[test]
fn windows_open_dir_rejects_a_reparse_point_in_an_intermediate_component() {
    let tmp = std::env::temp_dir().join(format!("rustils-r2-symlink-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");

    root.create_dir(OsStr::new("target")).expect("mkdir target");
    root.create_dir(OsStr::new("target/leaf"))
        .expect("mkdir target/leaf");
    match root.symlink(OsStr::new("target"), OsStr::new("link")) {
        Ok(()) => {
            let e = root
                .open_dir(OsStr::new("link/leaf"))
                .err()
                .expect("an intermediate reparse point must now be rejected under R2");
            assert_eq!(e.kind, ErrorKind::FilesystemLoop);

            // Sanity: the real, non-symlinked path still works.
            root.open_dir(OsStr::new("target/leaf"))
                .expect("the real, non-symlinked path must still open");
        }
        Err(e) if e.to_string().contains("1314") || e.to_string().contains("privilege") => {
            println!(
                "Skipping reparse-point containment test: SeCreateSymbolicLinkPrivilege \
                 (Win32 1314) not held by current user process"
            );
        }
        Err(e) => panic!("symlink failed with unexpected error: {:?}", e),
    }

    std::fs::remove_dir_all(&tmp).ok();
}

/// R2-equivalent containment, `create_dir`'s side: creating
/// `"link/newdir"` through an intermediate reparse point is rejected the
/// same way `open_dir` rejects it above, rather than silently creating
/// the directory on the other side of the link.
#[test]
fn windows_create_dir_rejects_a_reparse_point_in_an_intermediate_component() {
    let tmp = std::env::temp_dir().join(format!("rustils-r2-mkdir-symlink-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_windows::WindowsDir::open_ambient(&tmp).expect("open ambient");

    root.create_dir(OsStr::new("target")).expect("mkdir target");
    match root.symlink(OsStr::new("target"), OsStr::new("link")) {
        Ok(()) => {
            let e = root
                .create_dir(OsStr::new("link/newdir"))
                .expect_err("an intermediate reparse point must now be rejected under R2");
            assert_eq!(e.kind, ErrorKind::FilesystemLoop);
            assert!(
                root.metadata(OsStr::new("target/newdir")).is_err(),
                "the directory must not have been created on the other side of the link"
            );

            // A single-component create_dir is unaffected.
            root.create_dir(OsStr::new("plain"))
                .expect("plain mkdir still works");
        }
        Err(e) if e.to_string().contains("1314") || e.to_string().contains("privilege") => {
            println!(
                "Skipping reparse-point containment test: SeCreateSymbolicLinkPrivilege \
                 (Win32 1314) not held by current user process"
            );
        }
        Err(e) => panic!("symlink failed with unexpected error: {:?}", e),
    }

    std::fs::remove_dir_all(&tmp).ok();
}

/// Tun surface (RFC v2 R5+, D14): `wintun` has no backend yet — no
/// Windows consumer has named itself (rusty_tail's `ts-tun`, the only
/// named consumer for this surface, is Linux-only). `create` reports
/// `Unsupported` explicitly rather than the module being missing.
#[test]
fn windows_tun_create_is_unsupported() {
    use platform::tun::Tun;

    let tun = platform_windows::WindowsTun;
    let e = tun
        .create("ts0", std::net::Ipv4Addr::new(100, 64, 0, 1), 10, 1280)
        .err()
        .expect("must refuse: no wintun backend");
    assert_eq!(e.kind, platform::error::ErrorKind::Unsupported);
}
