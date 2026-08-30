//! Live verification for the raw-fd + non-blocking escape hatch
//! (rustils#41): concrete constructors, `AsFd`/`AsRawFd`, and
//! `set_nonblocking` on the five Linux socket types. Linux-only and
//! inherent-impl-only by design (see `net.rs`'s module doc) — not part
//! of the cross-backend `docs/behavior/net.md` spec or the shared
//! `net_parity.rs` suite, so it lives in its own file.
//!
//! `#![cfg(target_os = "linux")]`: integration test files don't inherit
//! the library crate's own `#![cfg(target_os = "linux")]` — each one
//! needs its own gate, or `cargo check --target x86_64-pc-windows-gnu
//! --all-targets` tries to build this file too, where `libc` isn't even
//! a dependency (target-gated in `Cargo.toml`). The exact mistake
//! `security_sandbox.rs` made and fixed first, per its own doc comment.
//!
//! `#![allow(unsafe_code)]`: this file makes raw `libc` calls directly
//! (bypassing `platform-linux`'s own code entirely) specifically to
//! verify the actual kernel-visible state a wrapper call produced, the
//! same live-verification discipline the CSPRNG/Sandbox work established
//! — not something this crate's own `sys/` layer needs to expose.
#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::fd::AsRawFd;

use platform::error::ErrorKind;
use platform::net::{TcpListener as _, TcpStream as _, UdpSocket as _};
use platform_linux::{LinuxTcpListener, LinuxTcpStream, LinuxUdpSocket};

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

/// `fcntl(F_GETFL)` read directly, bypassing this crate's own code —
/// the same "verify the actual kernel state, not just that our wrapper
/// returned `Ok`" discipline the Landlock/seccomp work this session
/// established.
fn is_nonblocking(fd: &impl AsRawFd) -> bool {
    // SAFETY: `fd` is a valid, open fd for the duration of this call.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0, "fcntl(F_GETFL) failed");
    flags & libc::O_NONBLOCK != 0
}

#[test]
fn concrete_tcp_constructors_bypass_the_boxed_trait() {
    let listener = LinuxTcpListener::bind(loopback(0)).expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    assert_ne!(addr.port(), 0);

    let handle = std::thread::spawn(move || {
        let (mut stream, _peer) = listener.accept().expect("accept");
        let mut buf = [0u8; 4];
        let n = stream.read(&mut buf).expect("server read");
        assert_eq!(&buf[..n], b"ping");
    });

    let mut client = LinuxTcpStream::connect(addr).expect("connect");
    client.write(b"ping").expect("client write");
    drop(client);
    handle.join().expect("server thread");
}

#[test]
fn set_nonblocking_actually_changes_kernel_flags_not_just_returns_ok() {
    let listener = LinuxTcpListener::bind(loopback(0)).expect("bind");
    assert!(
        !is_nonblocking(&listener),
        "a freshly bound listener starts blocking"
    );

    listener
        .set_nonblocking(true)
        .expect("set_nonblocking(true)");
    assert!(
        is_nonblocking(&listener),
        "O_NONBLOCK must actually be set in the kernel, not just claimed"
    );

    listener
        .set_nonblocking(false)
        .expect("set_nonblocking(false)");
    assert!(
        !is_nonblocking(&listener),
        "clearing O_NONBLOCK must actually clear it"
    );
}

#[test]
fn nonblocking_accept_with_no_pending_connection_returns_would_block_not_hang() {
    let listener = LinuxTcpListener::bind(loopback(0)).expect("bind");
    listener.set_nonblocking(true).expect("set_nonblocking");

    // No connection is pending — a blocking accept would hang forever;
    // this must return immediately with WouldBlock/EAGAIN.
    let err = match listener.accept() {
        Ok(_) => panic!("no connection was pending"),
        Err(e) => e,
    };
    assert_eq!(err.kind, ErrorKind::WouldBlock);
}

#[test]
fn nonblocking_udp_recv_with_no_datagram_returns_would_block_not_hang() {
    let sock = LinuxUdpSocket::bind(loopback(0)).expect("bind");
    sock.set_nonblocking(true).expect("set_nonblocking");

    let mut buf = [0u8; 16];
    let err = sock.recv_from(&mut buf).expect_err("nothing sent yet");
    assert_eq!(err.kind, ErrorKind::WouldBlock);
}

#[test]
fn as_raw_fd_reports_a_real_open_fd() {
    let sock = LinuxUdpSocket::bind(loopback(0)).expect("bind");
    let raw = sock.as_raw_fd();
    assert!(raw >= 0, "must be a real fd, not a sentinel");

    // Independent proof it's genuinely open and the kernel agrees it's a
    // socket: fstat via a raw libc call, not this crate's own code.
    // SAFETY: all-zeroes is a valid (if meaningless) `stat`, the same
    // reasoning `sys/net.rs`'s own `sockaddr_storage` zeroing relies on.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `raw` is a valid fd owned by `sock`, alive for this call;
    // `st` is a valid, exclusively borrowed out-param the kernel fills.
    let rc = unsafe { libc::fstat(raw, &mut st) };
    assert_eq!(rc, 0, "fstat on the reported fd must succeed");
    assert_eq!(
        st.st_mode & libc::S_IFMT,
        libc::S_IFSOCK,
        "must be a socket"
    );
}

#[test]
fn from_owned_fd_adopts_an_externally_created_socket() {
    // Built entirely through std, not this crate's own connect/bind path
    // — proves `From<OwnedFd>` genuinely adopts a foreign fd rather than
    // silently requiring one this crate created itself.
    let std_sock = std::net::UdpSocket::bind(loopback(0)).expect("std bind");
    let addr = std_sock.local_addr().expect("std local_addr");
    let fd: std::os::fd::OwnedFd = std_sock.into();

    let sock = LinuxUdpSocket::from(fd);
    let peer = LinuxUdpSocket::bind(loopback(0)).expect("bind peer");
    let peer_addr = peer.local_addr().expect("local_addr");

    peer.send_to(b"hello", addr).expect("send_to adopted sock");
    let mut buf = [0u8; 16];
    let (n, from) = sock.recv_from(&mut buf).expect("recv_from on adopted sock");
    assert_eq!(&buf[..n], b"hello");
    assert_eq!(from, peer_addr);
}
