//! Peer identity and owner-only socket files on Windows, the pieces
//! `rusty-mux-server` needs in place of `SO_PEERCRED` and `chmod 0600`:
//! `WindowsUnixStream::peer_pid`, `is_current_user`, `fs::restrict_to_owner`,
//! plus the std `Read`/`Write`/`set_read_timeout` surface its clients use.
//! Runs only on CI's Windows leg.

#![cfg(windows)]
#![allow(unsafe_code)]

use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use platform_windows::ffi::win32_surface as w;
use platform_windows::sys::handle::OwnedWinHandle;
use platform_windows::sys::security::UserSid;
use platform_windows::util::wide::to_wide_nul;
use platform_windows::{fs, is_current_user, WindowsUnixListener, WindowsUnixStream};

/// A socket path unique to this test, removed before use.
fn socket_path(name: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("pw-unix-peer-{name}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// A connected (server side, client side) pair over a real listener.
fn pair(path: &Path) -> (WindowsUnixListener, WindowsUnixStream, WindowsUnixStream) {
    let listener = WindowsUnixListener::bind(path).expect("bind");
    let client = WindowsUnixStream::connect(path).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    (listener, server, client)
}

/// The current user's SID as `whoami` prints it: an oracle independent
/// of the token code under test.
fn whoami_sid() -> String {
    let out = std::process::Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .expect("run whoami");
    assert!(out.status.success(), "whoami failed: {out:?}");
    let line = String::from_utf8_lossy(&out.stdout);
    // `"DOMAIN\user","S-1-5-21-..."`
    line.trim()
        .rsplit(',')
        .next()
        .expect("whoami output has a SID column")
        .trim_matches('"')
        .to_string()
}

/// One DACL entry as read back: type, access mask, SID string.
#[derive(Debug, PartialEq)]
struct Entry {
    kind: u8,
    mask: u32,
    sid: String,
}

/// `path`'s security-descriptor control flags and DACL entries, read with
/// `GetSecurityInfo` through a handle that does not follow the socket's
/// reparse point.
fn dacl_of(path: &Path) -> (u16, Vec<Entry>) {
    let wide = to_wide_nul(path.as_os_str());
    // SAFETY: `wide` is NUL-terminated and outlives the call; optional
    // pointers are null.
    let raw = unsafe {
        w::CreateFileW(
            wide.as_ptr(),
            w::READ_CONTROL,
            w::FILE_SHARE_READ | w::FILE_SHARE_WRITE | w::FILE_SHARE_DELETE,
            std::ptr::null(),
            w::OPEN_EXISTING,
            w::FILE_FLAG_OPEN_REPARSE_POINT | w::FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: a failure sentinel or a fresh handle nothing else owns.
    let file = unsafe { OwnedWinHandle::from_raw(raw) }.expect("open for READ_CONTROL");

    let mut acl: *mut w::ACL = std::ptr::null_mut();
    let mut sd: w::PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `file` is open with `READ_CONTROL`; only the DACL is asked
    // for, so the other out-pointers may be null; `sd` receives a
    // `LocalAlloc`ed descriptor that `acl` points into.
    let code = unsafe {
        w::GetSecurityInfo(
            file.as_raw(),
            w::SE_FILE_OBJECT,
            w::DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    assert_eq!(code, 0, "GetSecurityInfo");
    assert!(!acl.is_null(), "no DACL present");

    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: `sd` is the descriptor just returned; out-pointers are valid.
    let ok = unsafe { w::GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
    assert_ne!(ok, 0, "GetSecurityDescriptorControl");

    let mut info = w::ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: `acl` points into `sd`; `info` is the right size for the class.
    let ok = unsafe {
        w::GetAclInformation(
            acl,
            (&mut info as *mut w::ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<w::ACL_SIZE_INFORMATION>() as u32,
            w::AclSizeInformation,
        )
    };
    assert_ne!(ok, 0, "GetAclInformation");

    let entries = (0..info.AceCount)
        .map(|i| {
            let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY: `i` is below the ACL's entry count.
            assert_ne!(unsafe { w::GetAce(acl, i, &mut ace) }, 0, "GetAce");
            let ace = ace.cast::<w::ACCESS_ALLOWED_ACE>();
            // SAFETY: every ACE starts with an `ACE_HEADER` followed by its
            // mask; the SID of an access-allowed ACE starts at `SidStart`,
            // and it is only read as a SID once the type says it is one.
            let (kind, mask) = unsafe { ((*ace).Header.AceType, (*ace).Mask) };
            let sid = if u32::from(kind) == w::ACCESS_ALLOWED_ACE_TYPE {
                // SAFETY: see above.
                sid_string(unsafe { std::ptr::addr_of_mut!((*ace).SidStart).cast() })
            } else {
                String::new()
            };
            Entry { kind, mask, sid }
        })
        .collect();
    // SAFETY: `sd` came from `GetSecurityInfo`, whose contract is one
    // `LocalFree`; `acl` is not used after this.
    unsafe { w::LocalFree(sd) };
    (control, entries)
}

fn sid_string(sid: w::PSID) -> String {
    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` is a valid SID; `raw` receives a `LocalAlloc`ed string.
    assert_ne!(unsafe { w::ConvertSidToStringSidW(sid, &mut raw) }, 0);
    // SAFETY: NUL-terminated on success; copied before the free.
    let text = unsafe {
        let mut n = 0;
        while *raw.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(raw, n))
    };
    // SAFETY: allocated by `ConvertSidToStringSidW`; freed once.
    unsafe { w::LocalFree(raw.cast()) };
    text
}

#[test]
fn peer_pid_names_this_process_on_both_ends() {
    let path = socket_path("pid");
    let (_listener, server, client) = pair(&path);
    assert_eq!(
        server.peer_pid().expect("server peer_pid"),
        std::process::id()
    );
    assert_eq!(
        client.peer_pid().expect("client peer_pid"),
        std::process::id()
    );
}

/// Set in the re-exec'd helper's environment: the socket it connects to.
const HELPER_PATH_ENV: &str = "PW_UNIX_PEER_HELPER_PATH";
const HELPER_BUDGET: Duration = Duration::from_secs(15);

/// The helper half of `peer_pid_names_a_separate_client_process`: when
/// re-exec'd with `HELPER_PATH_ENV`, connect and hold the connection
/// until the server closes it. A no-op in a normal test run.
#[test]
fn cross_process_peer_helper() {
    let Some(path) = std::env::var_os(HELPER_PATH_ENV) else {
        return;
    };
    let mut stream = WindowsUnixStream::connect(Path::new(&path)).expect("helper connect");
    let _ = stream.read(&mut [0u8; 1]);
}

/// The peer pid and user check against a genuinely separate process:
/// this test binary re-exec'd as a client.
#[test]
fn peer_pid_names_a_separate_client_process() {
    let path = socket_path("cross-process");
    let listener = WindowsUnixListener::bind(&path).expect("bind");
    let (accepted, accept) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = accepted.send(listener.accept().map(|(stream, _)| stream));
    });

    let mut child = std::process::Command::new(std::env::current_exe().expect("current_exe"))
        .args([
            "--exact",
            "cross_process_peer_helper",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HELPER_PATH_ENV, &path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn helper");

    let server = match accept.recv_timeout(HELPER_BUDGET) {
        Ok(stream) => stream.expect("accept"),
        Err(_) => {
            let _ = child.kill();
            panic!("the helper did not connect within {HELPER_BUDGET:?}");
        }
    };
    let peer = server.peer_pid().expect("peer_pid");
    let same_user = is_current_user(peer);
    drop(server);

    let deadline = Instant::now() + HELPER_BUDGET;
    while child.try_wait().expect("try_wait").is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("the helper did not exit after the server closed");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(peer, child.id(), "peer_pid must name the client process");
    assert_ne!(peer, std::process::id());
    assert!(same_user.expect("is_current_user(helper)"));
}

#[test]
fn is_current_user_accepts_this_process() {
    assert!(is_current_user(std::process::id()).expect("own pid"));
}

#[test]
fn is_current_user_fails_for_a_missing_process() {
    // Windows pids are multiples of 4 well below this.
    assert!(is_current_user(0xFFFF_FFF0).is_err());
}

#[test]
fn is_current_user_does_not_accept_the_system_process() {
    // Pid 4 is System: either its token can't be read (Err) or its user
    // is LocalSystem, never the test's user.
    assert!(!matches!(is_current_user(4), Ok(true)));
}

/// A process of another user that every Windows host has: the service
/// control manager runs as LocalSystem. Proving that a *different
/// interactive user* is refused needs a second account and stays a
/// manual check (CUJ 3 in the design); this is the automated part. On an
/// elevated runner the token is readable and the answer is `Ok(false)`;
/// otherwise it is `Err`. Both are a refusal.
#[test]
fn is_current_user_refuses_a_process_of_another_user() {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq services.exe", "/FO", "CSV", "/NH"])
        .output()
        .expect("run tasklist");
    let text = String::from_utf8_lossy(&out.stdout);
    // `"services.exe","712","Services","0","9,876 K"`
    let pid: u32 = text
        .lines()
        .find(|l| l.starts_with('"'))
        .and_then(|l| l.split(',').nth(1))
        .map(|f| f.trim_matches('"').parse().expect("pid column"))
        .unwrap_or_else(|| panic!("services.exe not listed: {text:?}"));
    let answer = is_current_user(pid);
    eprintln!("is_current_user(services.exe = {pid}) = {answer:?}");
    assert!(!matches!(answer, Ok(true)));
}

#[test]
fn user_sid_matches_whoami() {
    let sid = UserSid::current()
        .expect("current")
        .to_sid_string()
        .expect("string");
    assert_eq!(sid, whoami_sid());
}

#[test]
fn restrict_to_owner_leaves_one_protected_full_access_entry_for_this_user() {
    let path = socket_path("dacl");
    let listener = WindowsUnixListener::bind(&path).expect("bind");
    fs::restrict_to_owner(&path).expect("restrict_to_owner");

    let (control, entries) = dacl_of(&path);
    assert_eq!(
        entries,
        vec![Entry {
            kind: w::ACCESS_ALLOWED_ACE_TYPE as u8,
            mask: w::FILE_ALL_ACCESS,
            sid: whoami_sid(),
        }],
        "the DACL must hold exactly one full-access allow entry, for the current user"
    );
    assert_ne!(
        control & w::SE_DACL_PROTECTED,
        0,
        "the DACL must be protected from inherited entries"
    );

    // The owner can still connect.
    let mut client = WindowsUnixStream::connect(&path).expect("connect after restrict");
    let (mut server, _) = listener.accept().expect("accept");
    client.write_all(b"ok").expect("write");
    let mut buf = [0u8; 2];
    server.read_exact(&mut buf).expect("read");
    assert_eq!(&buf, b"ok");
}

#[test]
fn std_read_and_write_round_trip() {
    let path = socket_path("io");
    let (_listener, mut server, mut client) = pair(&path);
    client.write_all(b"hello").expect("client write");
    let mut buf = [0u8; 5];
    server.read_exact(&mut buf).expect("server read");
    assert_eq!(&buf, b"hello");
    server.write_all(b"back").expect("server write");
    let mut buf = [0u8; 4];
    client.read_exact(&mut buf).expect("client read");
    assert_eq!(&buf, b"back");
}

/// Reads once from `stream` on a thread of its own, so a read that never
/// ends fails the test after `budget` instead of hanging it.
fn read_within(mut stream: WindowsUnixStream, budget: Duration) -> std::io::Result<usize> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(stream.read(&mut [0u8; 8]));
    });
    rx.recv_timeout(budget)
        .expect("the read did not return within its budget")
}

fn assert_timed_out(result: std::io::Result<usize>) {
    let err = result.expect_err("no data was sent");
    assert!(
        matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock),
        "unexpected error kind: {err:?}"
    );
}

#[test]
fn read_timeout_ends_a_read_with_no_data() {
    let path = socket_path("timeout");
    let (_listener, _server, client) = pair(&path);
    client
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("set_read_timeout");
    let started = Instant::now();
    assert_timed_out(read_within(client, Duration::from_secs(5)));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn a_zero_read_timeout_is_refused() {
    let path = socket_path("timeout-zero");
    let (_listener, _server, client) = pair(&path);
    let err = client
        .set_read_timeout(Some(Duration::ZERO))
        .expect_err("zero must be refused, as std does");
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

/// A positive timeout under 1 ms must not become Winsock's 0, which means
/// "wait forever".
#[test]
fn a_sub_millisecond_read_timeout_still_times_out() {
    let path = socket_path("timeout-ns");
    let (_listener, _server, client) = pair(&path);
    client
        .set_read_timeout(Some(Duration::from_nanos(1)))
        .expect("set_read_timeout");
    assert_timed_out(read_within(client, Duration::from_secs(5)));
}

/// `None` clears an earlier timeout: a read then waits past it for data.
#[test]
fn a_none_read_timeout_waits_for_data() {
    let path = socket_path("timeout-none");
    let (_listener, mut server, client) = pair(&path);
    client
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set_read_timeout");
    client.set_read_timeout(None).expect("clear the timeout");
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        server.write_all(b"late").expect("write");
        server
    });
    let n = read_within(client, Duration::from_secs(10)).expect("read waits for the data");
    assert!(n > 0);
    drop(writer.join().expect("writer"));
}

#[test]
fn a_clone_shares_the_connection_both_ways() {
    let path = socket_path("clone");
    let (_listener, mut server, mut client) = pair(&path);
    let mut clone = client.try_clone().expect("try_clone");
    server.write_all(b"to clone").expect("server write");
    let mut buf = [0u8; 8];
    clone.read_exact(&mut buf).expect("clone read");
    assert_eq!(&buf, b"to clone");
    clone.write_all(b"from it").expect("clone write");
    let mut buf = [0u8; 7];
    server.read_exact(&mut buf).expect("server read");
    assert_eq!(&buf, b"from it");
    // The original still works alongside the clone.
    client.write_all(b"!").expect("original write");
    let mut one = [0u8; 1];
    server.read_exact(&mut one).expect("server read");
    assert_eq!(&one, b"!");
}

/// The attach client's detach: its output thread is blocked reading a
/// clone, the input side shuts the original down, and the server closes
/// its end once it reads end of stream. The read must end. This does not
/// cover a server that never closes (a hung server).
#[test]
fn shutdown_ends_a_read_blocked_on_a_clone() {
    let path = socket_path("shutdown");
    let (_listener, mut server, client) = pair(&path);
    let peer = std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        while matches!(server.read(&mut buf), Ok(n) if n > 0) {}
        drop(server);
    });
    let clone = client.try_clone().expect("try_clone");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut clone = clone;
        let _ = tx.send(clone.read(&mut [0u8; 8]));
    });
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    client.shutdown(std::net::Shutdown::Both).expect("shutdown");
    let ended = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the clone's read did not end within 2 s of the shutdown");
    println!(
        "clone read ended after {:?} with {ended:?}",
        started.elapsed()
    );
    assert!(matches!(ended, Ok(0) | Err(_)), "{ended:?}");
    peer.join().expect("peer");
}

#[test]
fn writing_after_shutdown_fails() {
    let path = socket_path("shutdown-write");
    let (_listener, _server, mut client) = pair(&path);
    client
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown");
    assert!(client.write_all(b"late").is_err());
}
