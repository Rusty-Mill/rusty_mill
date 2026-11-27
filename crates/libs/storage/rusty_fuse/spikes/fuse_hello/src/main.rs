//! Phase 0 spike B (`fuse_hello`): a read-only hello filesystem spoken over raw
//! `/dev/fuse`, with no libfuse and no third-party crate (D7, D8).
//!
//! Throwaway, test-exempt spike code (QA.md § Testing). Missing wrappers
//! (`mount`, `umount2`, `unshare`) stay spike-local via `rusty_libc::arch`
//! syscalls (D13, D15).
//!
//! Modes:
//! - `run <mnt>`    mount, serve on a thread, self-test with std::fs, unmount.
//! - `serve <mnt>`  mount and serve in the foreground until unmounted.
//! - `umount <mnt>` umount2(mnt, 0).
//! - `userns <mnt>` unshare(CLONE_NEWUSER|CLONE_NEWNS), map ids, then `run`.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::{Arc, Mutex};

use rusty_libc::arch::{from_ret, syscall2, syscall5, Errno};
use rusty_libc::fd::{self, IoSlice};

// x86_64 syscall numbers not wrapped by rusty_libc (spike-local, D15).
const SYS_STATFS: usize = 137;
const SYS_MOUNT: usize = 165;
const SYS_UMOUNT2: usize = 166;
const SYS_UNSHARE: usize = 272;

const MS_NOSUID: usize = 2;
const MS_NODEV: usize = 4;
const CLONE_NEWNS: usize = 0x0002_0000;
const CLONE_NEWUSER: usize = 0x1000_0000;

// include/uapi/linux/fuse.h opcodes.
const LOOKUP: u32 = 1;
const FORGET: u32 = 2;
const GETATTR: u32 = 3;
const OPEN: u32 = 14;
const READ: u32 = 15;
const STATFS: u32 = 17;
const RELEASE: u32 = 18;
const INIT: u32 = 26;
const OPENDIR: u32 = 27;
const READDIR: u32 = 28;
const RELEASEDIR: u32 = 29;
const INTERRUPT: u32 = 36;
const DESTROY: u32 = 38;
const BATCH_FORGET: u32 = 42;

const ENOENT: i32 = 2;
const ENOSYS: i32 = 38;
const ENOTDIR: i32 = 20;
const EISDIR: i32 = 21;

const IN_HEADER: usize = 40;
const OUR_MINOR: u32 = 31;
const MAX_WRITE: u32 = 128 * 1024;

const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const DT_DIR: u32 = 4;
const DT_REG: u32 = 8;

/// The fixed in-memory tree, the same as spike A: `/hello.txt`, `/dir/a.txt`.
struct Node {
    ino: u64,
    parent: u64,
    name: &'static str,
    data: Option<&'static [u8]>, // None = directory
}

const TREE: &[Node] = &[
    Node { ino: 1, parent: 1, name: "", data: None },
    Node { ino: 2, parent: 1, name: "hello.txt", data: Some(b"Hello from rusty_fuse on Linux\n") },
    Node { ino: 3, parent: 1, name: "dir", data: None },
    Node { ino: 4, parent: 3, name: "a.txt", data: Some(b"a\n") },
];

fn node(ino: u64) -> Option<&'static Node> {
    TREE.iter().find(|n| n.ino == ino)
}

/// Opcode name -> count, plus negotiated INIT values, for the evidence log.
#[derive(Default)]
struct Evidence {
    ops: BTreeMap<String, u32>,
    kernel_version: Option<(u32, u32)>,
    negotiated_minor: Option<u32>,
    end_reason: String,
}

type Shared = Arc<Mutex<Evidence>>;

fn op_name(op: u32) -> String {
    let s = match op {
        LOOKUP => "LOOKUP", FORGET => "FORGET", GETATTR => "GETATTR", OPEN => "OPEN",
        READ => "READ", STATFS => "STATFS", RELEASE => "RELEASE", INIT => "INIT",
        OPENDIR => "OPENDIR", READDIR => "READDIR", RELEASEDIR => "RELEASEDIR",
        INTERRUPT => "INTERRUPT", DESTROY => "DESTROY", BATCH_FORGET => "BATCH_FORGET",
        other => return format!("op{other}(ENOSYS)"),
    };
    s.to_string()
}

// ---------- little-endian helpers ----------

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"))
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"))
}
fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn put64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// `struct fuse_attr`, 88 bytes.
fn attr(n: &Node, uid: u32, gid: u32) -> Vec<u8> {
    let (mode, size, nlink) = match n.data {
        Some(d) => (S_IFREG | 0o444, d.len() as u64, 1),
        None => (S_IFDIR | 0o555, 0, 2),
    };
    let mut v = Vec::with_capacity(88);
    put64(&mut v, n.ino);
    put64(&mut v, size);
    put64(&mut v, size.div_ceil(512)); // blocks
    for _ in 0..3 {
        put64(&mut v, 0); // atime, mtime, ctime
    }
    for _ in 0..3 {
        put32(&mut v, 0); // *timensec
    }
    put32(&mut v, mode);
    put32(&mut v, nlink);
    put32(&mut v, uid);
    put32(&mut v, gid);
    put32(&mut v, 0); // rdev
    put32(&mut v, 4096); // blksize
    put32(&mut v, 0); // flags
    debug_assert_eq!(v.len(), 88);
    v
}

/// `struct fuse_entry_out`, 128 bytes.
fn entry_out(n: &Node, uid: u32, gid: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(128);
    put64(&mut v, n.ino); // nodeid
    put64(&mut v, 0); // generation
    put64(&mut v, 1); // entry_valid (s)
    put64(&mut v, 1); // attr_valid (s)
    put32(&mut v, 0);
    put32(&mut v, 0);
    v.extend(attr(n, uid, gid));
    v
}

/// `struct fuse_attr_out`, 104 bytes.
fn attr_out(n: &Node, uid: u32, gid: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(104);
    put64(&mut v, 1);
    put32(&mut v, 0);
    put32(&mut v, 0);
    v.extend(attr(n, uid, gid));
    v
}

/// `struct fuse_open_out`, 16 bytes.
fn open_out(fh: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(16);
    put64(&mut v, fh);
    put32(&mut v, 0);
    put32(&mut v, 0);
    v
}

/// `struct fuse_init_out`, 64 bytes.
fn init_out(minor: u32, max_readahead: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(64);
    put32(&mut v, 7);
    put32(&mut v, minor);
    put32(&mut v, max_readahead);
    put32(&mut v, 0); // flags: no optional features
    v.extend_from_slice(&16u16.to_le_bytes()); // max_background
    v.extend_from_slice(&12u16.to_le_bytes()); // congestion_threshold
    put32(&mut v, MAX_WRITE);
    put32(&mut v, 1); // time_gran
    v.extend_from_slice(&32u16.to_le_bytes()); // max_pages
    v.extend_from_slice(&0u16.to_le_bytes()); // map_alignment
    put32(&mut v, 0); // flags2
    put32(&mut v, 0); // max_stack_depth
    v.resize(64, 0); // unused[6]
    v
}

/// `struct fuse_kstatfs`, 80 bytes.
fn statfs_out() -> Vec<u8> {
    let mut v = Vec::with_capacity(80);
    for x in [16u64, 0, 0, TREE.len() as u64, 0] {
        put64(&mut v, x); // blocks bfree bavail files ffree
    }
    for x in [4096u32, 255, 4096, 0] {
        put32(&mut v, x); // bsize namelen frsize padding
    }
    v.resize(80, 0);
    v
}

/// READDIR payload: `struct fuse_dirent` records, each padded to 8 bytes,
/// starting after `offset` and fitting in `size`.
fn readdir(dir: u64, offset: u64, size: usize) -> Vec<u8> {
    let mut entries: Vec<(u64, &str, u32)> = vec![(dir, ".", DT_DIR), (node(dir).map_or(1, |n| n.parent), "..", DT_DIR)];
    for n in TREE.iter().filter(|n| n.parent == dir && n.ino != dir) {
        entries.push((n.ino, n.name, if n.data.is_some() { DT_REG } else { DT_DIR }));
    }
    let mut v = Vec::new();
    for (i, (ino, name, ty)) in entries.iter().enumerate().skip(offset as usize) {
        let reclen = (24 + name.len()).div_ceil(8) * 8;
        if v.len() + reclen > size {
            break;
        }
        put64(&mut v, *ino);
        put64(&mut v, i as u64 + 1); // offset of the next entry
        put32(&mut v, name.len() as u32);
        put32(&mut v, *ty);
        v.extend_from_slice(name.as_bytes());
        v.resize(v.len() + (reclen - 24 - name.len()), 0);
    }
    v
}

fn reply(fd: i32, unique: u64, error: i32, payload: &[u8]) -> Result<(), Errno> {
    let len = (16 + payload.len()) as u32;
    let mut hdr = Vec::with_capacity(16);
    put32(&mut hdr, len);
    hdr.extend_from_slice(&(-error).to_le_bytes());
    put64(&mut hdr, unique);
    let bufs = [IoSlice::new(&hdr), IoSlice::new(payload)];
    fd::writev(fd, &bufs).map(|_| ())
}

/// The single-threaded request loop. Returns when the kernel ends the session.
fn serve(fd: i32, ev: Shared) {
    let (uid, gid) = (rusty_libc::process::getuid(), rusty_libc::process::getgid());
    let mut buf = vec![0u8; MAX_WRITE as usize + 4096];
    loop {
        let n = match fd::read(fd, &mut buf) {
            Ok(n) => n,
            Err(e) if e.0 == 4 || e.0 == ENOENT => continue, // EINTR, interrupted request
            Err(e) if e.0 == 19 => {
                ev.lock().expect("lock").end_reason = "read: ENODEV (unmounted)".into();
                return;
            }
            Err(e) => {
                ev.lock().expect("lock").end_reason = format!("read error {e:?}");
                return;
            }
        };
        if n < IN_HEADER {
            continue;
        }
        let req = &buf[..n];
        let (op, unique, nodeid) = (u32_at(req, 4), u64_at(req, 8), u64_at(req, 16));
        let arg = &req[IN_HEADER..];
        *ev.lock().expect("lock").ops.entry(op_name(op)).or_default() += 1;

        let res = match op {
            FORGET | BATCH_FORGET | INTERRUPT => continue, // no reply allowed
            INIT => {
                let (major, minor, ra) = (u32_at(arg, 0), u32_at(arg, 4), u32_at(arg, 8));
                let ours = minor.min(OUR_MINOR);
                let mut e = ev.lock().expect("lock");
                e.kernel_version = Some((major, minor));
                e.negotiated_minor = Some(ours);
                drop(e);
                reply(fd, unique, 0, &init_out(ours, ra))
            }
            LOOKUP => {
                let name = arg.split(|b| *b == 0).next().unwrap_or_default();
                match TREE.iter().find(|n| n.parent == nodeid && n.ino != nodeid && n.name.as_bytes() == name) {
                    Some(n) => reply(fd, unique, 0, &entry_out(n, uid, gid)),
                    None => reply(fd, unique, ENOENT, &[]),
                }
            }
            GETATTR => match node(nodeid) {
                Some(n) => reply(fd, unique, 0, &attr_out(n, uid, gid)),
                None => reply(fd, unique, ENOENT, &[]),
            },
            OPENDIR => match node(nodeid) {
                Some(n) if n.data.is_none() => reply(fd, unique, 0, &open_out(nodeid)),
                Some(_) => reply(fd, unique, ENOTDIR, &[]),
                None => reply(fd, unique, ENOENT, &[]),
            },
            READDIR => {
                let (off, size) = (u64_at(arg, 8), u32_at(arg, 16) as usize);
                reply(fd, unique, 0, &readdir(nodeid, off, size))
            }
            OPEN => match node(nodeid) {
                Some(n) if n.data.is_some() => reply(fd, unique, 0, &open_out(nodeid)),
                Some(_) => reply(fd, unique, EISDIR, &[]),
                None => reply(fd, unique, ENOENT, &[]),
            },
            READ => {
                let (off, size) = (u64_at(arg, 8) as usize, u32_at(arg, 16) as usize);
                let data = node(nodeid).and_then(|n| n.data).unwrap_or_default();
                let start = off.min(data.len());
                let end = (start + size).min(data.len());
                reply(fd, unique, 0, &data[start..end])
            }
            RELEASE | RELEASEDIR => reply(fd, unique, 0, &[]),
            STATFS => reply(fd, unique, 0, &statfs_out()),
            DESTROY => {
                let r = reply(fd, unique, 0, &[]);
                ev.lock().expect("lock").end_reason = "DESTROY".into();
                let _ = r;
                return;
            }
            _ => reply(fd, unique, ENOSYS, &[]),
        };
        if let Err(e) = res {
            // ENOENT on write means the request was interrupted; log anything else.
            if e.0 != ENOENT {
                eprintln!("reply to op {op} failed: {e:?}");
            }
        }
    }
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("no NUL in path")
}

fn mount_fuse(mnt: &str, fd: i32, uid: u32, gid: u32) -> Result<(), Errno> {
    let (src, target, fstype) = (cstr("rusty_fuse_spike"), cstr(mnt), cstr("fuse.rusty_fuse_spike"));
    let opts = cstr(&format!("fd={fd},rootmode=40000,user_id={uid},group_id={gid}"));
    // SAFETY: all pointers are valid NUL-terminated strings that outlive the call.
    let ret = unsafe {
        syscall5(
            SYS_MOUNT,
            src.as_ptr() as usize,
            target.as_ptr() as usize,
            fstype.as_ptr() as usize,
            MS_NOSUID | MS_NODEV,
            opts.as_ptr() as usize,
        )
    };
    from_ret(ret).map(|_| ())
}

fn umount(mnt: &str) -> Result<(), Errno> {
    let target = cstr(mnt);
    // SAFETY: valid NUL-terminated path.
    from_ret(unsafe { syscall2(SYS_UMOUNT2, target.as_ptr() as usize, 0) }).map(|_| ())
}

fn statfs_type(mnt: &str) -> Result<u64, Errno> {
    let target = cstr(mnt);
    let mut buf = [0u8; 120]; // struct statfs on x86_64
    // SAFETY: buf is large enough for struct statfs.
    from_ret(unsafe { syscall2(SYS_STATFS, target.as_ptr() as usize, buf.as_mut_ptr() as usize) })?;
    Ok(u64_at(&buf, 0))
}

fn open_dev() -> Result<i32, Errno> {
    fd::open(c"/dev/fuse", fd::O_RDWR | fd::O_CLOEXEC, 0)
}

fn print_ev(label: &str, ev: &Shared) {
    let e = ev.lock().expect("lock");
    println!("{label} kernel FUSE protocol: {:?}, negotiated 7.{:?}", e.kernel_version, e.negotiated_minor);
    println!("{label} opcodes seen: {:?}", e.ops);
    println!("{label} serve loop ended: {}", e.end_reason);
}

/// Mount, serve on a thread, check the tree through the kernel, unmount.
fn run(mnt: &str, label: &str) -> bool {
    let (uid, gid) = (rusty_libc::process::getuid(), rusty_libc::process::getgid());
    let fd = match open_dev() {
        Ok(fd) => fd,
        Err(e) => {
            println!("{label} open /dev/fuse: FAIL {e:?} (missing or denied device)");
            return false;
        }
    };
    println!("{label} open /dev/fuse: ok fd={fd}");
    if let Err(e) = mount_fuse(mnt, fd, uid, gid) {
        println!("{label} mount(2): FAIL {e:?} (EPERM = no CAP_SYS_ADMIN in this ns or seccomp/LSM denial)");
        return false;
    }
    println!("{label} mount(2): ok at {mnt} as uid={uid}");
    let ev: Shared = Arc::default();
    let worker = {
        let ev = ev.clone();
        std::thread::spawn(move || serve(fd, ev))
    };

    let mut go = true;
    let mut check = |what: &str, ok: bool, detail: String| {
        println!("{label} {what}: {} {detail}", if ok { "ok" } else { "FAIL" });
        go &= ok;
    };
    let mut names: Vec<String> = std::fs::read_dir(mnt)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    check("ls /", names == ["dir", "hello.txt"], format!("{names:?}"));
    let hello = std::fs::read_to_string(format!("{mnt}/hello.txt"));
    check("cat hello.txt", hello.as_deref().is_ok_and(|s| s.starts_with("Hello")), format!("{hello:?}"));
    let a = std::fs::read_to_string(format!("{mnt}/dir/a.txt"));
    check("cat dir/a.txt", a.as_deref().is_ok_and(|s| s == "a\n"), format!("{a:?}"));
    let missing = std::fs::metadata(format!("{mnt}/nope"));
    check("stat missing -> ENOENT", missing.as_ref().err().and_then(|e| e.raw_os_error()) == Some(ENOENT), String::new());
    let st = statfs_type(mnt);
    check("statfs f_type == FUSE_SUPER_MAGIC", st == Ok(0x6573_5546), format!("{st:x?}"));

    let um = umount(mnt);
    check("umount2", um.is_ok(), format!("{um:?}"));
    if worker.join().is_err() {
        check("serve thread", false, "panicked".into());
    }
    let _ = fd::close(fd);
    print_ev(label, &ev);
    println!("{label} GO: {}", if go { "yes" } else { "no" });
    go
}

fn write_file(path: &str, s: &str) -> Result<(), std::io::Error> {
    std::fs::write(path, s)
}

/// Q5: become root in a new user namespace with a private mount namespace,
/// then try the same mount as `run`.
fn userns(mnt: &str) -> bool {
    let (uid, gid) = (rusty_libc::process::getuid(), rusty_libc::process::getgid());
    println!("[userns] outer uid={uid} gid={gid}");
    // SAFETY: plain syscall with flag arguments; called before any thread exists.
    let r = from_ret(unsafe { syscall2(SYS_UNSHARE, CLONE_NEWUSER | CLONE_NEWNS, 0) });
    if let Err(e) = r {
        println!("[userns] unshare(CLONE_NEWUSER|CLONE_NEWNS): FAIL {e:?} (EPERM/ENOSPC = userns disabled or restricted)");
        return false;
    }
    println!("[userns] unshare(CLONE_NEWUSER|CLONE_NEWNS): ok");
    let maps = write_file("/proc/self/setgroups", "deny")
        .and_then(|_| write_file("/proc/self/uid_map", &format!("0 {uid} 1")))
        .and_then(|_| write_file("/proc/self/gid_map", &format!("0 {gid} 1")));
    if let Err(e) = maps {
        println!("[userns] uid/gid map: FAIL {e}");
        return false;
    }
    println!("[userns] uid/gid map: ok, inner uid={}", rusty_libc::process::getuid());
    run(mnt, "[userns]")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(mode), Some(mnt)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: fuse_hello run|serve|umount|userns <mountpoint>");
        std::process::exit(2);
    };
    let ok = match mode.as_str() {
        "run" => run(mnt, "[root]"),
        "userns" => userns(mnt),
        "umount" => umount(mnt).map_err(|e| eprintln!("umount2: {e:?}")).is_ok(),
        "serve" => {
            let (uid, gid) = (rusty_libc::process::getuid(), rusty_libc::process::getgid());
            match open_dev().and_then(|fd| mount_fuse(mnt, fd, uid, gid).map(|_| fd)) {
                Ok(fd) => {
                    let ev: Shared = Arc::default();
                    println!("[serve] mounted {mnt}; waiting for umount");
                    serve(fd, ev.clone());
                    print_ev("[serve]", &ev);
                    true
                }
                Err(e) => {
                    println!("[serve] open/mount: FAIL {e:?}");
                    false
                }
            }
        }
        _ => false,
    };
    std::process::exit(if ok { 0 } else { 1 });
}
