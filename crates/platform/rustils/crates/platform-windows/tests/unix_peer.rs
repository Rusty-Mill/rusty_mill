//! Peer identity and owner-only socket files on Windows, the pieces
//! `rusty-mux-server` needs in place of `SO_PEERCRED` and `chmod 0600`:
//! `WindowsUnixStream::peer_pid`, `is_current_user`, `fs::restrict_to_owner`,
//! plus the std `Read`/`Write`/`set_read_timeout` surface its clients use.
//! Runs only on CI's Windows leg.

#![cfg(windows)]
#![allow(unsafe_code)]

use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
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

/// `(ace_type, sid_string)` for every entry of `path`'s DACL, read
/// through a handle that does not follow the socket's reparse point.
fn dacl_entries(path: &Path) -> Vec<(u8, String)> {
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

    let mut len = 0u32;
    // SAFETY: a null buffer of length 0 asks only for the size.
    unsafe {
        w::GetKernelObjectSecurity(
            file.as_raw(),
            w::DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut len,
        )
    };
    assert!(len > 0, "GetKernelObjectSecurity reported no size");
    let mut sd = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `sd` is writable for at least `len` bytes.
    let ok = unsafe {
        w::GetKernelObjectSecurity(
            file.as_raw(),
            w::DACL_SECURITY_INFORMATION,
            sd.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    };
    assert_ne!(ok, 0, "GetKernelObjectSecurity");

    let mut present = 0;
    let mut defaulted = 0;
    let mut acl: *mut w::ACL = std::ptr::null_mut();
    // SAFETY: `sd` holds the descriptor just read; out-pointers are valid.
    let ok = unsafe {
        w::GetSecurityDescriptorDacl(
            sd.as_mut_ptr().cast(),
            &mut present,
            &mut acl,
            &mut defaulted,
        )
    };
    assert!(ok != 0 && present != 0 && !acl.is_null(), "no DACL present");

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

    (0..info.AceCount)
        .map(|i| {
            let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
            // SAFETY: `i` is below the ACL's entry count.
            assert_ne!(unsafe { w::GetAce(acl, i, &mut ace) }, 0, "GetAce");
            let ace = ace.cast::<w::ACCESS_ALLOWED_ACE>();
            // SAFETY: every ACE starts with an `ACE_HEADER`; the SID of an
            // access-allowed ACE starts at `SidStart`. Only read as a SID
            // once the type says it is one.
            let kind = unsafe { (*ace).Header.AceType };
            let sid = if u32::from(kind) == w::ACCESS_ALLOWED_ACE_TYPE {
                // SAFETY: see above.
                sid_string(unsafe { std::ptr::addr_of_mut!((*ace).SidStart).cast() })
            } else {
                String::new()
            };
            (kind, sid)
        })
        .collect()
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

#[test]
fn user_sid_matches_whoami() {
    let sid = UserSid::current()
        .expect("current")
        .to_sid_string()
        .expect("string");
    assert_eq!(sid, whoami_sid());
}

#[test]
fn restrict_to_owner_leaves_one_entry_for_this_user() {
    let path = socket_path("dacl");
    let listener = WindowsUnixListener::bind(&path).expect("bind");
    fs::restrict_to_owner(&path).expect("restrict_to_owner");

    let entries = dacl_entries(&path);
    assert_eq!(
        entries,
        vec![(w::ACCESS_ALLOWED_ACE_TYPE as u8, whoami_sid())],
        "the DACL must hold exactly one allow entry, for the current user"
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

#[test]
fn read_timeout_ends_a_read_with_no_data() {
    let path = socket_path("timeout");
    let (_listener, _server, mut client) = pair(&path);
    client
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("set_read_timeout");
    let started = Instant::now();
    let err = client.read(&mut [0u8; 8]).expect_err("no data was sent");
    assert!(
        matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock),
        "unexpected error kind: {err:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
}
