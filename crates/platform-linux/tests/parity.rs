//! Parity suite seed (RFC v2 §9): one behavior-spec-derived assertion set,
//! run against every backend available on this host. On Linux CI that is
//! `platform-mock` + `platform-linux`; the Windows leg will add
//! `platform-windows` when its Dir impl lands (R1). The suite living as a
//! generic function over `&dyn Dir` is the point — a backend passes or it
//! doesn't, with no backend-specific test text to drift.

use std::ffi::OsStr;

use platform::error::ErrorKind;
use platform::fs::{AccessMode, Dir, FileType, OpenOptions};

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
    // `linux_metadata_reports_a_real_nlink_mtime_and_permissions`.
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
    root.symlink(OsStr::new("nowhere/dangling"), OsStr::new("link1"))
        .expect("symlink");
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

    // A symlink to an existing directory (Windows divergence,
    // `Dir::symlink`'s doc comment: the NT reparse point must declare
    // file-vs-directory at creation, decided here by `target` existing
    // as a directory relative to `self`). Cleanup tries `remove_file`
    // first, falling back to `remove_dir` — the same "try file, then
    // directory" shape `rename`'s own implementation already uses,
    // since which one Windows requires for a directory-type link isn't
    // pinned by this suite.
    root.create_dir(OsStr::new("realdir"))
        .expect("mkdir realdir");
    root.symlink(OsStr::new("realdir"), OsStr::new("dirlink"))
        .expect("symlink to dir");
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
    root.remove_dir(OsStr::new("realdir")).expect("rm realdir");

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

#[cfg(target_os = "linux")]
#[test]
fn linux_backend_conforms() {
    let tmp = std::env::temp_dir().join(format!("rustils-parity-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    assert_fs_behavior(&root);
    std::fs::remove_dir_all(&tmp).ok();
}

/// Divergence #005 (docs/divergences.md), Linux side: a plain file we
/// create has no execute permission bit (`OpenOptions::create_truncate`
/// mode `0o666`, no `x` for anyone regardless of umask), and `access`
/// reports that honestly. Windows has no such bit to check — its own
/// pinning test (`windows_access_grants_execute_unconditionally`)
/// asserts the opposite for the identical setup.
#[cfg(target_os = "linux")]
#[test]
fn linux_access_denies_execute_on_a_plain_file() {
    let tmp = std::env::temp_dir().join(format!("rustils-access-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    root.open(OsStr::new("f"), &OpenOptions::create_truncate())
        .expect("create f");
    let e = root
        .access(OsStr::new("f"), AccessMode::execute())
        .expect_err("a plain data file has no execute bit");
    assert_eq!(e.kind, ErrorKind::PermissionDenied);
    std::fs::remove_dir_all(&tmp).ok();
}

/// `Metadata::nlink`/`modified` and `UnixMode::permissions` (coreutils
/// gap backlog #63/#65, `ls -l` donor material), checked against a raw
/// `libc::stat` call issued directly by the test — bypassing this
/// crate's own code entirely, the same "verify the actual kernel state,
/// not just that our wrapper returned a number" discipline this
/// session's Tun/Sandbox work already established — rather than
/// asserting a specific value this test would have to predict (e.g. the
/// exact permission bits after the ambient umask is applied).
#[cfg(target_os = "linux")]
#[test]
fn linux_metadata_reports_a_real_nlink_mtime_and_permissions() {
    use std::os::unix::ffi::OsStrExt;

    let tmp = std::env::temp_dir().join(format!("rustils-metadata-ext-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    root.open(OsStr::new("f"), &OpenOptions::create_truncate())
        .expect("create f");

    let md = root.metadata(OsStr::new("f")).expect("metadata");
    let unix_mode = root
        .unix_mode(OsStr::new("f"))
        .expect("unix_mode")
        .expect("Linux always answers Some");

    let path = tmp.join("f");
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no interior NUL");
    // SAFETY: `st` is a plain-old-data struct for which all-zeroes is a
    // valid (if meaningless) value, overwritten by the kernel on
    // success; `c_path` is a valid, NUL-terminated path outliving the
    // call.
    #[allow(unsafe_code)]
    let st: libc::stat = unsafe {
        let mut st = std::mem::zeroed();
        let rc = libc::stat(c_path.as_ptr(), &mut st);
        assert_eq!(rc, 0, "raw stat must succeed");
        st
    };

    assert_eq!(md.nlink, st.st_nlink, "nlink must match a raw stat");
    assert_eq!(
        unix_mode.permissions,
        (st.st_mode & 0o777) as u16,
        "permissions must match a raw stat"
    );
    let expected_mtime = std::time::SystemTime::UNIX_EPOCH
        + std::time::Duration::new(st.st_mtime as u64, st.st_mtime_nsec as u32);
    assert_eq!(md.modified, expected_mtime, "mtime must match a raw stat");

    std::fs::remove_dir_all(&tmp).ok();
}

/// `Dir::set_unix_mode` (coreutils gap backlog #64, `unix_mode`'s
/// write-side companion), checked against a raw `libc::stat` call issued
/// directly by the test, same discipline as
/// `linux_metadata_reports_a_real_nlink_mtime_and_permissions`. Not part
/// of the shared `assert_fs_behavior`: this is a real backend divergence
/// (Windows is `Unsupported`; `platform-mock` is a documented no-op), the
/// same situation `linux_access_denies_execute_on_a_plain_file` /
/// `windows_access_grants_execute_unconditionally` are in.
#[cfg(all(target_os = "linux", not(feature = "track-p")))]
#[test]
fn linux_chmod_sets_real_permission_and_special_bits() {
    use platform::fs::Mode;
    use std::os::unix::ffi::OsStrExt;

    let tmp = std::env::temp_dir().join(format!("rustils-chmod-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    root.open(OsStr::new("f"), &OpenOptions::create_truncate())
        .expect("create f");

    root.set_unix_mode(
        OsStr::new("f"),
        Mode {
            setuid: true,
            setgid: false,
            sticky: true,
            permissions: 0o640,
        },
    )
    .expect("chmod must succeed");

    let path = tmp.join("f");
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("no interior NUL");
    // SAFETY: see `linux_metadata_reports_a_real_nlink_mtime_and_permissions`.
    #[allow(unsafe_code)]
    let st: libc::stat = unsafe {
        let mut st = std::mem::zeroed();
        let rc = libc::stat(c_path.as_ptr(), &mut st);
        assert_eq!(rc, 0, "raw stat must succeed");
        st
    };
    assert_ne!(st.st_mode & libc::S_ISUID, 0, "setuid must be set");
    assert_eq!(st.st_mode & libc::S_ISGID, 0, "setgid must be clear");
    assert_ne!(st.st_mode & libc::S_ISVTX, 0, "sticky must be set");
    assert_eq!(
        (st.st_mode & 0o777) as u16,
        0o640,
        "permission bits must match a raw stat"
    );

    // Confirmed via `Dir::unix_mode` too, not just the raw stat above.
    let unix_mode = root
        .unix_mode(OsStr::new("f"))
        .expect("unix_mode")
        .expect("Linux always answers Some");
    assert!(unix_mode.setuid);
    assert!(!unix_mode.setgid);
    assert!(unix_mode.sticky);
    assert_eq!(unix_mode.permissions, 0o640);

    std::fs::remove_dir_all(&tmp).ok();
}

/// Track P (RFC v2 §2): `rusty_libc` has no `chmod`/`fchmodat` binding
/// yet at the pinned rev — `Dir::set_unix_mode` is `Unsupported` under
/// this feature rather than silently falling back to libc, pinned here
/// so the track-p CI leg has real coverage of this arm instead of
/// silently skipping it (`sys/fdio.rs::set_unix_mode`'s track-p arm).
#[cfg(all(target_os = "linux", feature = "track-p"))]
#[test]
fn linux_chmod_is_unsupported_under_track_p() {
    use platform::fs::Mode;

    let tmp = std::env::temp_dir().join(format!("rustils-chmod-trackp-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    root.open(OsStr::new("f"), &OpenOptions::create_truncate())
        .expect("create f");

    let e = root
        .set_unix_mode(OsStr::new("f"), Mode::default())
        .expect_err("rusty_libc has no chmod primitive yet");
    assert_eq!(e.kind, ErrorKind::Unsupported);

    std::fs::remove_dir_all(&tmp).ok();
}

/// Process behavior (docs/behavior/process.md) against the native
/// backend. Fixtures are OS-specific (`sh`); the assertions mirror the
/// Windows leg's.
#[cfg(target_os = "linux")]
#[test]
fn linux_process_backend_conforms() {
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    use platform::process::{Command, EnvSpec, ExitStatus, Spawner, Stdio};

    let tmp = std::env::temp_dir().join(format!("rustils-proc-parity-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let s = platform_linux::LinuxSpawner;

    // Exit codes decode uniformly.
    let c = Command::new("sh", tmp.clone()).arg("-c").arg("exit 7");
    let child = s.spawn(&c).expect("spawn");
    assert!(child.id() > 0);
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(7));

    // Signal termination decodes as Signaled — the B-5 sentinel: a raw
    // waitpid status word (9 vs 0x0009 vs 35072) must never leak through.
    let c = Command::new("sh", tmp.clone()).arg("-c").arg("kill -9 $$");
    assert_eq!(
        s.spawn(&c).expect("spawn").wait().expect("wait"),
        ExitStatus::Signaled(9)
    );

    // cwd is honored: the child sees the marker only if it starts there.
    std::fs::write(tmp.join("rustils-marker"), b"x").expect("marker");
    let c = Command::new("sh", tmp.clone())
        .arg("-c")
        .arg("test -e rustils-marker");
    assert!(s.spawn(&c).expect("spawn").wait().expect("wait").success());

    // Explicit env starts empty: only the given variables are visible.
    let mut env = BTreeMap::new();
    env.insert(OsString::from("RUSTILS_PARITY"), OsString::from("yes"));
    let mut c = Command::new("sh", tmp.clone())
        .arg("-c")
        .arg("test \"$RUSTILS_PARITY\" = yes && test -z \"$RUSTILS_ABSENT\"");
    c.env = EnvSpec::Explicit(env);
    assert!(s.spawn(&c).expect("spawn").wait().expect("wait").success());

    // Stdio::Null wiring spawns and completes.
    let mut c = Command::new("sh", tmp.clone())
        .arg("-c")
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

/// Groups and kill-tree (divergence 001 pin, Linux side). The sleeper
/// would run 30s; the test completing inside cargo's normal budget IS the
/// kill assertion (wall-clock discipline, extraction map D7).
#[cfg(target_os = "linux")]
#[test]
fn linux_process_group_kill() {
    use platform::process::{Command, ExitStatus, GroupSpec, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;

    // kill_tree on a NewGroup child: SIGKILL reaches the group; wait
    // reports Signaled(9) — never a raw status word (B-5).
    let c = Command::new("sh", tmp.clone())
        .arg("-c")
        .arg("sleep 30")
        .group(GroupSpec::NewGroup);
    let child = s.spawn(&c).expect("spawn");
    child.kill_tree(Signal::Kill).expect("kill_tree");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Signaled(9));

    // kill_tree reaches a grandchild: the shell spawns its own child and
    // waits on it; killing the group takes both down promptly.
    let c = Command::new("sh", tmp.clone())
        .arg("-c")
        .arg("sleep 30 & wait")
        .group(GroupSpec::NewGroup);
    let child = s.spawn(&c).expect("spawn");
    std::thread::sleep(std::time::Duration::from_millis(100));
    child.kill_tree(Signal::Kill).expect("kill_tree");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Signaled(9));

    // kill_single works without a group.
    let c = Command::new("sh", tmp.clone()).arg("-c").arg("sleep 30");
    let child = s.spawn(&c).expect("spawn");
    child.kill_single(Signal::Kill).expect("kill_single");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Signaled(9));

    // kill_tree without NewGroup is refused, not guessed at.
    let c = Command::new("sh", tmp).arg("-c").arg("exit 0");
    let child = s.spawn(&c).expect("spawn");
    assert_eq!(
        child.kill_tree(Signal::Kill).expect_err("must refuse").kind,
        platform::error::ErrorKind::Unsupported
    );
    child.wait().expect("wait");
}

/// try_wait + wait_any (extraction map step 3 seed, Linux side). The
/// 30s sleeper finishing inside cargo's budget IS the assertion that
/// wait_any picked the quick child, not the sleeper.
#[cfg(target_os = "linux")]
#[test]
fn linux_wait_any_and_try_wait() {
    use platform::process::{wait_any, Child, Command, ExitStatus, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;

    // try_wait: None while running; Some after; wait returns the stashed
    // status (WNOHANG reaped the zombie — losing it would hang or error).
    let c = Command::new("sh", tmp.clone()).arg("-c").arg("exit 4");
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
    let sleeper = s
        .spawn(&Command::new("sh", tmp.clone()).arg("-c").arg("sleep 30"))
        .expect("spawn");
    let quick = s
        .spawn(&Command::new("sh", tmp.clone()).arg("-c").arg("exit 9"))
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

    // Empty set is refused like the OS primitives refuse it — by both
    // the portable loop and the backend multiplexer.
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

/// Many children through the multiplexer (R3): 70 processes collected to
/// completion. On Linux this exercises a 70-fd poll set; the count is
/// chosen to force the Windows leg past WaitForMultipleObjects's
/// 64-handle cap into the chunked sweep.
#[cfg(target_os = "linux")]
#[test]
fn linux_wait_any_many_children() {
    use platform::process::{Child, Command, ExitStatus, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let mut children: Vec<Box<dyn Child>> = (0..70)
        .map(|i| {
            s.spawn(
                &Command::new("sh", tmp.clone())
                    .arg("-c")
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

/// Pipes (extraction map step 4, Linux side): capture, stdin feeding with
/// EOF-by-drop, and pipe-read EOF when the child exits.
#[cfg(target_os = "linux")]
#[test]
fn linux_process_pipes() {
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;

    // stdout capture: bytes arrive, then EOF at child exit.
    // \377 (octal) — POSIX printf; \x escapes are a bashism dash lacks.
    let mut c = Command::new("sh", tmp.clone())
        .arg("-c")
        .arg("printf 'cap\\377 tured'");
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
    assert_eq!(got, b"cap\xff tured");
    assert!(child.wait().expect("wait").success());

    // stdin feeding: write, drop for EOF, child echoes through its own
    // piped stdout.
    let mut c = Command::new("cat", tmp);
    c.stdin = Stdio::Pipe;
    c.stdout = Stdio::Pipe;
    let mut child = s.spawn(&c).expect("spawn");
    let mut stdin = child.take_stdin().expect("piped stdin");
    let mut stdout = child.take_stdout().expect("piped stdout");
    stdin.write(b"round trip").expect("write");
    drop(stdin); // EOF — without this, cat never exits (D5 contract)
    let mut got = Vec::new();
    loop {
        let n = stdout.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, b"round trip");
    assert!(child.wait().expect("wait").success());
}

/// Deferred signals (D6, Linux side): a child delivers SIGTERM to this
/// test process; the handler's one atomic store is consumed at the next
/// safe point. Also pins take-consumes and burst coalescing semantics.
#[cfg(target_os = "linux")]
#[test]
fn linux_signal_source_defers_and_coalesces() {
    use platform::events::{SignalEvent, SignalSource};
    use platform::process::{Command, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let signals = platform_linux::LinuxSignalSource;
    signals
        .install(&[SignalEvent::Terminate, SignalEvent::Hangup])
        .expect("install");
    assert_eq!(signals.take(), None);

    // A real delivery: the child TERMs its parent (this process), which
    // must survive (deferral, not default disposition) and observe the
    // event at this safe point.
    let c = Command::new("sh", tmp.clone())
        .arg("-c")
        .arg("kill -TERM $PPID");
    assert!(s.spawn(&c).expect("spawn").wait().expect("wait").success());
    let mut seen = None;
    for _ in 0..100 {
        if let Some(e) = signals.take() {
            seen = Some(e);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(seen, Some(SignalEvent::Terminate));
    assert_eq!(signals.take(), None, "take consumes");

    // Burst: HUP then TERM before a take — single slot, latest wins.
    let c = Command::new("sh", tmp)
        .arg("-c")
        .arg("kill -HUP $PPID; kill -TERM $PPID");
    assert!(s.spawn(&c).expect("spawn").wait().expect("wait").success());
    let mut seen = None;
    for _ in 0..100 {
        if let Some(e) = signals.take() {
            seen = Some(e);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // Two rapid deliveries may coalesce to the latest (single slot) or,
    // if this process consumed between them, arrive as just the first —
    // what is pinned: at least one arrives and it is one of the two.
    assert!(matches!(
        seen,
        Some(SignalEvent::Terminate) | Some(SignalEvent::Hangup)
    ));
}

/// The redirected-streams contract (`docs/behavior/term.md`): under a
/// test harness no std stream is a terminal, and the surface must say
/// so honestly — false everywhere, Err for size, refuse raw mode, and
/// keep leave_raw an Ok no-op.
#[cfg(target_os = "linux")]
#[test]
fn linux_terminal_honest_when_redirected() {
    use platform::term::{TermStream, Terminal};
    let mut t = platform_linux::LinuxTerminal::new();
    // cargo's test harness captures the streams; none is a tty.
    assert!(!t.is_tty(TermStream::Stdin));
    assert!(!t.is_tty(TermStream::Stdout));
    assert!(!t.is_tty(TermStream::Stderr));
    assert!(t.window_size().is_err(), "no tty: size must be Err");
    assert!(!t.is_raw(), "no tty: is_raw's live probe must say false");
    assert!(t.enter_raw().is_err(), "no tty: raw mode must refuse");
    t.leave_raw().expect("leave without enter is an Ok no-op");
    assert!(t.set_echo(false).is_err(), "no tty: set_echo must refuse");
}

/// Portable `Signal` (rustils#46, D1's `kill_cmd`): every non-`Kill`
/// identity actually reaches the child as the matching raw signal,
/// decoded through the existing `Signaled` arm — the B-5 sentinel again,
/// just with a caller-chosen signal instead of the hardcoded SIGKILL.
#[cfg(target_os = "linux")]
#[test]
fn linux_kill_signal_is_portable() {
    use platform::process::{Command, ExitStatus, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;

    for (sig, raw) in [
        (Signal::Term, 15),
        (Signal::Int, 2),
        (Signal::Hup, 1),
        (Signal::Quit, 3),
    ] {
        let c = Command::new("sh", tmp.clone()).arg("-c").arg("sleep 30");
        let child = s.spawn(&c).expect("spawn");
        child.kill_single(sig).expect("kill_single");
        assert_eq!(child.wait().expect("wait"), ExitStatus::Signaled(raw));
    }
}

/// `GroupSpec::JoinGroup` (rustils#44, D1's pipeline shape): a second
/// stage joins the first stage's already-created group instead of
/// leading its own — `kill_tree` on the joiner reaches the leader too,
/// which is only possible if the join actually placed it in that pgid
/// (a broken join would leave the second stage in the test process's own
/// group, invisible to a kill targeted at the leader's pgid).
#[cfg(target_os = "linux")]
#[test]
fn linux_process_group_join() {
    use platform::process::{Command, ExitStatus, GroupSpec, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;

    let leader = s
        .spawn(
            &Command::new("sh", tmp.clone())
                .arg("-c")
                .arg("sleep 30")
                .group(GroupSpec::NewGroup),
        )
        .expect("spawn leader");
    let pgid = leader.id();

    let follower = s
        .spawn(
            &Command::new("sh", tmp)
                .arg("-c")
                .arg("sleep 30")
                .group(GroupSpec::JoinGroup(pgid)),
        )
        .expect("spawn follower");

    // kill_tree on the *follower* targets the *leader's* pgid (the join
    // target it was given, not its own pid) — reaching the leader is the
    // proof the join landed in the right group.
    follower.kill_tree(Signal::Kill).expect("kill_tree");
    assert_eq!(follower.wait().expect("wait"), ExitStatus::Signaled(9));
    assert_eq!(leader.wait().expect("wait"), ExitStatus::Signaled(9));
}

/// `Spawner::adopt` (rustils#47): POSIX `setpgid(pid, pgid)` can only
/// retarget a process that is both the caller's own child and has not
/// yet exec'd — by the time any caller has a pid to adopt, that's never
/// true, so this is `Unsupported` on Unix (divergence 010), never
/// attempted speculatively. Uses a real spawned child's pid so the
/// refusal is provably about the operation, not a bogus pid.
#[cfg(target_os = "linux")]
#[test]
fn linux_adopt_is_unsupported() {
    use platform::process::{Command, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let child = s
        .spawn(&Command::new("sh", tmp).arg("-c").arg("sleep 30"))
        .expect("spawn");
    let pid = child.id();

    let e = s
        .adopt(pid)
        .err()
        .expect("setpgid cannot retarget a foreign pid on Unix");
    assert_eq!(e.kind, ErrorKind::Unsupported);

    child.kill_single(Signal::Kill).expect("cleanup kill");
    child.wait().expect("cleanup wait");
}

/// `Child::wait_job`/`try_wait_job` (rustils#45, D10): the
/// `WUNTRACED`/`WCONTINUED` half of wait — a child observed stopping,
/// then continuing, then finally exiting, none of which the plain
/// `wait`/`try_wait` pair (no `WUNTRACED`/`WCONTINUED`) can ever see.
#[cfg(target_os = "linux")]
#[test]
fn linux_wait_job_observes_stop_and_continue() {
    use platform::process::{Command, ExitStatus, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;

    // The `sleep 0.2` between resuming and exiting is load-bearing, not
    // padding: without it, a fast scheduler can run the resumed shell
    // straight through to `exit 5` before the very next line's
    // `wait_job()` call observes the continued transition at all — the
    // kernel's wait notification collapses straight to the exit status
    // once the child is already a zombie, silently skipping `Continued`.
    // The sleep keeps the child alive and running long enough that the
    // continued transition is reliably still pending when `wait_job()`
    // (called within microseconds of the `SIGCONT` below) looks for it.
    let c = Command::new("sh", tmp)
        .arg("-c")
        .arg("kill -STOP $$; sleep 0.2; exit 5");
    let mut child = s.spawn(&c).expect("spawn");

    // Blocking: the child stops itself shortly after spawn.
    assert_eq!(child.wait_job().expect("wait_job"), ExitStatus::Stopped(19));

    // Non-blocking: nothing new until we resume it.
    assert_eq!(child.try_wait_job().expect("try_wait_job"), None);
    child.kill_single(Signal::Cont).expect("SIGCONT");
    assert_eq!(child.wait_job().expect("wait_job"), ExitStatus::Continued);

    // The eventual exit is still a terminal, stashed result — reachable
    // through either the job-aware or the plain wait/try_wait pair.
    assert_eq!(child.wait_job().expect("wait_job"), ExitStatus::Code(5));
    assert_eq!(
        child.try_wait_job().expect("repoll"),
        Some(ExitStatus::Code(5))
    );
    assert_eq!(child.wait().expect("wait"), ExitStatus::Code(5));
}

/// `Spawner::is_alive` (`docs/decision-request-detach-liveness.md`):
/// `Ok(true)` while a real child is running, `Ok(false)` once it has
/// actually exited and been reaped.
#[cfg(target_os = "linux")]
#[test]
fn linux_is_alive_reports_running_then_exited() {
    use platform::process::{Command, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let c = Command::new("sh", tmp).arg("-c").arg("sleep 30");
    let child = s.spawn(&c).expect("spawn");
    let pid = child.id();

    assert!(
        s.is_alive(pid).expect("probe"),
        "must be alive while running"
    );
    child.kill_single(Signal::Kill).expect("kill_single");
    child.wait().expect("wait");
    assert!(
        !s.is_alive(pid).expect("probe"),
        "must not be alive once reaped"
    );
}

/// `Spawner::is_zombie`: `Ok(false)` for a running process, `Ok(true)`
/// for one that has terminated but not yet been reaped, `Ok(false)`
/// again once `try_wait` reaps it (the `/proc` entry is gone entirely).
#[cfg(target_os = "linux")]
#[test]
fn linux_is_zombie_detects_an_unreaped_child() {
    use platform::process::{Command, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let c = Command::new("sh", tmp).arg("-c").arg("sleep 30");
    let mut child = s.spawn(&c).expect("spawn");
    let pid = child.id();

    assert!(
        !s.is_zombie(pid).expect("probe"),
        "a running process is not a zombie"
    );
    child.kill_single(Signal::Kill).expect("kill_single");
    // The kernel needs a moment to transition the killed process to `Z`
    // before this, its real parent, reaps it — the probe above this
    // sleep must run before `try_wait` below does the reaping.
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        s.is_zombie(pid).expect("probe"),
        "an unreaped, terminated child is a zombie"
    );

    child.try_wait().expect("try_wait reaps it");
    assert!(
        !s.is_zombie(pid).expect("probe"),
        "a reaped pid no longer exists"
    );
}

/// `Command::detach`: `setsid` alone already gives the child its own
/// session and process group (`pid == pgid`) without an explicit
/// `GroupSpec::NewGroup` — `kill_tree` has a sound target purely from
/// `detach()`.
#[cfg(target_os = "linux")]
#[test]
fn linux_detach_gives_its_own_process_group() {
    use platform::process::{Command, ExitStatus, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let c = Command::new("sh", tmp).arg("-c").arg("sleep 30").detach();
    let child = s.spawn(&c).expect("spawn a detached child");

    child
        .kill_tree(Signal::Kill)
        .expect("kill_tree targets the detached child's own pgid");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Signaled(9));
}

/// `Command::detach` + `GroupSpec::NewGroup` composes cleanly on Linux
/// (divergence 015's Linux side): `NewGroup`'s own `setpgid(0, 0)` is
/// harmless self-targeting on top of `setsid`'s already-fresh pgid, so
/// the combination spawns successfully rather than conflicting.
#[cfg(target_os = "linux")]
#[test]
fn linux_detach_composes_with_new_group() {
    use platform::process::{Command, ExitStatus, GroupSpec, Signal, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let c = Command::new("sh", tmp)
        .arg("-c")
        .arg("sleep 30")
        .group(GroupSpec::NewGroup)
        .detach();
    let child = s.spawn(&c).expect("detach composes with NewGroup on Linux");
    child.kill_tree(Signal::Kill).expect("kill_tree");
    assert_eq!(child.wait().expect("wait"), ExitStatus::Signaled(9));
}

/// `Command::detach` + `GroupSpec::JoinGroup` is refused
/// (`InvalidInput`, self-contradictory — `setsid` starting a brand-new
/// session cannot also join an existing, different pgid).
#[cfg(target_os = "linux")]
#[test]
fn linux_detach_with_join_group_is_refused() {
    use platform::process::{Command, GroupSpec, Spawner};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let c = Command::new("sh", tmp)
        .arg("-c")
        .arg("exit 0")
        .group(GroupSpec::JoinGroup(1))
        .detach();
    assert_eq!(
        s.spawn(&c).err().expect("must refuse").kind,
        ErrorKind::InvalidInput
    );
}

/// `JobControlTerminal::give_terminal` (rustils#43, D1's `tcsetpgrp`
/// give/reclaim): under cargo's non-tty harness this refuses with the
/// same `ENOTTY` shape `enter_raw`/`window_size` already pin on a
/// redirected stdin (`docs/behavior/term.md`). The real give/reclaim
/// round-trip over a live controlling terminal needs a pty and is
/// live-verified only, not parity-pinned (same discipline as
/// `poll_readable`/`read_chunk`'s batching pin).
#[cfg(target_os = "linux")]
#[test]
fn linux_give_terminal_honest_when_redirected() {
    use platform::term::JobControlTerminal;

    let t = platform_linux::LinuxTerminal::new();
    assert!(
        t.give_terminal(std::process::id()).is_err(),
        "no controlling terminal: give_terminal must refuse"
    );
}

/// `Stdio::File` (rustils#51, D5): a stage's stdin/stdout wired to an
/// already-open `File` instead of `Inherit`/`Null`/`Pipe` — the `< file`
/// and `> file` shell-redirect shapes.
#[cfg(target_os = "linux")]
#[test]
fn linux_stdio_file_wiring() {
    use platform::fs::{Dir, OpenOptions};
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir().join(format!("rustils-stdio-file-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let dir = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    let s = platform_linux::LinuxSpawner;

    // `< file`: stdin wired to an already-open File, read back through a
    // piped stdout to prove the child actually saw the file's bytes.
    let mut in_file = dir
        .open(OsStr::new("in.txt"), &OpenOptions::create_truncate())
        .expect("create in.txt");
    in_file.write(b"hello from file").expect("write in.txt");
    drop(in_file);
    let in_read = dir
        .open(OsStr::new("in.txt"), &OpenOptions::read())
        .expect("reopen in.txt for read");

    let mut c = Command::new("cat", tmp.clone());
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
    assert_eq!(got, b"hello from file");
    assert!(child.wait().expect("wait").success());

    // `> file`: stdout wired to an already-open File, read back directly
    // from the filesystem afterward.
    let out_file = dir
        .open(OsStr::new("out.txt"), &OpenOptions::create_truncate())
        .expect("create out.txt");
    let mut c = Command::new("sh", tmp.clone())
        .arg("-c")
        .arg("printf 'to file'");
    c.stdout = Stdio::File(out_file);
    let child = s.spawn(&c).expect("spawn");
    assert!(child.wait().expect("wait").success());
    let mut readback = dir
        .open(OsStr::new("out.txt"), &OpenOptions::read())
        .expect("reopen out.txt");
    let mut got = [0u8; 64];
    let n = readback.read(&mut got).expect("read out.txt");
    assert_eq!(&got[..n], b"to file");

    std::fs::remove_dir_all(&tmp).ok();
}

/// `File::try_clone` + `Stdio::File` (rustils#51, D5): the `2>&1`/
/// `&> file` shape — stdout and stderr wired to *clones* of the same
/// open file, which must share the file's offset the way a real `dup2`
/// pair does, so sequential writes through either fd append rather than
/// clobber each other at position 0.
#[cfg(target_os = "linux")]
#[test]
fn linux_stdio_file_try_clone_shares_offset_for_dup_style_redirect() {
    use platform::fs::{Dir, OpenOptions};
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir().join(format!("rustils-stdio-dup-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let dir = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    let s = platform_linux::LinuxSpawner;

    let out_file = dir
        .open(OsStr::new("both.txt"), &OpenOptions::create_truncate())
        .expect("create both.txt");
    let err_file = out_file.try_clone().expect("try_clone");

    let mut c = Command::new("sh", tmp.clone())
        .arg("-c")
        .arg("printf 'out-' >&1; printf 'err-' >&2");
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
        got, b"out-err-",
        "shared offset: sequential writes append, not clobber"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

/// `Stdio::File` refuses a `File` from a foreign backend (rustils#51):
/// `LinuxSpawner` can only extract a raw fd from its own `LinuxFile`, so
/// a `platform-mock`-backed `File` fails `Unsupported` rather than the
/// spawn silently ignoring the redirect or panicking.
#[cfg(target_os = "linux")]
#[test]
fn linux_stdio_file_refuses_a_foreign_backend_file() {
    use platform::fs::{Dir, OpenOptions};
    use platform::process::{Command, Spawner, Stdio};

    let tmp = std::env::temp_dir();
    let s = platform_linux::LinuxSpawner;
    let foreign = platform_mock::MockDir::root()
        .with_file("f", b"x".to_vec())
        .open(OsStr::new("f"), &OpenOptions::read())
        .expect("mock open");

    let mut c = Command::new("cat", tmp);
    c.stdin = Stdio::File(foreign);
    let e = s
        .spawn(&c)
        .err()
        .expect("must refuse: foreign File backend");
    assert_eq!(e.kind, platform::error::ErrorKind::Unsupported);
}

/// D2 (`docs/behavior/fs.md`): `write_atomic`'s publishing `rename` is
/// followed by an `fsync` on the *directory* fd itself (`LinuxDir::
/// sync_dir`) — not just the temp file's own pre-rename `fsync` (D1),
/// which says nothing about whether the rename's own directory-entry
/// mutation survives a crash. Ordering/timing claims like this have no
/// portable value to assert in the shared parity suite — the existing
/// "fsync fires before renameat2" claim in `docs/behavior/fs.md` is
/// pinned the same way: verified live via `strace`. Unlike that
/// claim (a one-time development-session trace), this is a committed,
/// re-runnable proof: this very test binary re-executes itself under
/// `strace`, with an env-var guard selecting the traced half, and then
/// parses the syscall log for the actual ordering.
#[cfg(target_os = "linux")]
#[test]
fn write_atomic_fsyncs_the_directory_after_the_publishing_rename() {
    use platform::fs::Dir;

    const INNER: &str = "RUSTILS_STRACE_INNER_DIRSYNC";
    const INNER_DIR: &str = "RUSTILS_STRACE_INNER_DIRSYNC_DIR";

    if std::env::var_os(INNER).is_some() {
        // Traced half: exactly one write_atomic call, then exit.
        // Everything strace captured in this process belongs to it.
        let tmp = std::path::PathBuf::from(std::env::var_os(INNER_DIR).expect("dir env set"));
        let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
        root.write_atomic(OsStr::new("atomic.txt"), b"payload")
            .expect("write_atomic");
        return;
    }

    if std::process::Command::new("strace")
        .arg("-V")
        .output()
        .is_err()
    {
        eprintln!("skipping: strace not available in this environment");
        return;
    }

    let tmp = std::env::temp_dir().join(format!("rustils-dirsync-strace-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let trace_path = tmp.join("trace.log");
    let this_exe = std::env::current_exe().expect("current_exe");

    let output = std::process::Command::new("strace")
        .args(["-f", "-e", "trace=fsync,renameat2", "-o"])
        .arg(&trace_path)
        .arg(&this_exe)
        .args([
            "--exact",
            "write_atomic_fsyncs_the_directory_after_the_publishing_rename",
            "--nocapture",
        ])
        .env(INNER, "1")
        .env(INNER_DIR, &tmp)
        .output()
        .expect("spawn strace");

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() && stderr.contains("ptrace") {
        // Environment lacks ptrace permission (e.g. a container without
        // CAP_SYS_PTRACE / restrictive Yama ptrace_scope) — an
        // environment gap, not a code bug, the same honest-skip
        // discipline `tests/tun_parity.rs`'s `tun_or_skip!` uses for
        // its own CAP_NET_ADMIN gap.
        eprintln!("skipping: strace could not attach in this environment: {stderr}");
        return;
    }
    assert!(
        output.status.success(),
        "traced inner run must itself pass: {stderr}"
    );

    let trace = std::fs::read_to_string(&trace_path).expect("read trace log");
    let renameat2_pos = trace
        .find("renameat2(")
        .unwrap_or_else(|| panic!("renameat2 must appear in the trace:\n{trace}"));
    let after_rename = &trace[renameat2_pos..];
    assert!(
        after_rename.contains("fsync("),
        "no fsync call appears after the publishing renameat2 in the trace \
         (directory durability step missing or reordered):\n{trace}"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

/// R2 containment (`docs/behavior/fs.md`): `open_dir` now rejects a
/// symlink sitting in an intermediate path component instead of
/// silently following it — `sys::fdio::openat_r2`'s `RESOLVE_NO_SYMLINKS`
/// request, previously unenforced (plain `openat` follows every
/// component transparently, R1). Skips honestly on a pre-5.6 kernel,
/// where `openat_r2` falls back to the old R1 behavior by design and
/// this assertion would otherwise fail for the wrong reason.
#[cfg(target_os = "linux")]
#[test]
fn linux_open_dir_rejects_a_symlink_in_an_intermediate_component() {
    if !platform_linux::sys::fdio::openat2_supported() {
        eprintln!("skipping: openat2 unsupported on this kernel (pre-5.6)");
        return;
    }

    let tmp = std::env::temp_dir().join(format!("rustils-r2-symlink-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");

    // real/target exists as a real directory; "link" is a symlink to it.
    // "link/leaf" then has a symlink as its *intermediate* component.
    root.create_dir(OsStr::new("target")).expect("mkdir target");
    root.create_dir(OsStr::new("target/leaf"))
        .expect("mkdir target/leaf");
    root.symlink(OsStr::new("target"), OsStr::new("link"))
        .expect("symlink link -> target");

    let e = root
        .open_dir(OsStr::new("link/leaf"))
        .err()
        .expect("an intermediate symlink must now be rejected under R2");
    assert_eq!(e.kind, ErrorKind::FilesystemLoop);

    // Sanity: the same path through the real (non-symlinked) name still
    // works — this is a containment gain, not a general regression.
    root.open_dir(OsStr::new("target/leaf"))
        .expect("the real, non-symlinked path must still open");

    std::fs::remove_dir_all(&tmp).ok();
}

/// R2 containment, `create_dir`'s multi-component parent-resolution
/// path: creating `"link/newdir"` through an intermediate symlink is
/// rejected the same way `open_dir` rejects it above, rather than
/// silently creating the directory on the other side of the link.
#[cfg(target_os = "linux")]
#[test]
fn linux_create_dir_rejects_a_symlink_in_an_intermediate_component() {
    if !platform_linux::sys::fdio::openat2_supported() {
        eprintln!("skipping: openat2 unsupported on this kernel (pre-5.6)");
        return;
    }

    let tmp = std::env::temp_dir().join(format!("rustils-r2-mkdir-symlink-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");

    root.create_dir(OsStr::new("target")).expect("mkdir target");
    root.symlink(OsStr::new("target"), OsStr::new("link"))
        .expect("symlink link -> target");

    let e = root
        .create_dir(OsStr::new("link/newdir"))
        .expect_err("an intermediate symlink must now be rejected under R2");
    assert_eq!(e.kind, ErrorKind::FilesystemLoop);
    assert!(
        root.metadata(OsStr::new("target/newdir")).is_err(),
        "the directory must not have been created on the other side of the symlink"
    );

    // A single-component create_dir (the common case, no intermediate
    // component at all) is unaffected — always was, still is, R2.
    root.create_dir(OsStr::new("plain"))
        .expect("plain mkdir still works");

    std::fs::remove_dir_all(&tmp).ok();
}
