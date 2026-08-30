//! Live verification for the raw-socket + non-blocking escape hatch
//! (rustils#59, mirroring `platform-linux/tests/net_nonblocking.rs`
//! for rustils#41): concrete constructors, `AsRawSocket`, and
//! `set_nonblocking` on the five Windows socket types. Windows-only and
//! inherent-impl-only by design (see `net.rs`'s module doc) — not part
//! of the cross-backend `docs/behavior/net.md` spec or the shared
//! `net_parity.rs` suite, so it lives in its own file, same as the
//! Linux copy.
//!
//! `#![cfg(windows)]`: integration test files don't inherit the
//! library crate's own per-module `#[cfg(windows)]` gates — this one
//! needs its own, or `cross-compile-check`'s Linux-hosted
//! `--target x86_64-pc-windows-gnu` build would try to compile this
//! file's raw Winsock calls against a target it's already
//! cross-compiling for (harmless there), but a native Linux `cargo
//! test --workspace` run would fail to find `windows_sys` at all. The
//! exact mistake `platform-linux`'s own test files made and fixed
//! first (see that crate's `security_sandbox.rs`/`net_nonblocking.rs`
//! doc comments).
//!
//! Unlike the Linux copy, there is no direct "read back the flag"
//! query for Winsock's non-blocking mode (`ioctlsocket(FIONBIO, ...)`
//! is set-only) — verification here is behavioral (a would-otherwise-
//! block call returns `WouldBlock` immediately instead of hanging),
//! the same proof-by-effect the Linux suite's own
//! `nonblocking_accept_with_no_pending_connection_returns_would_block_not_hang`
//! test already uses alongside its flag-based one.
//!
//! `#![allow(unsafe_code)]`: this file makes one raw `windows_sys` call
//! directly (bypassing this crate's own code entirely) specifically to
//! verify the actual OS-visible state a wrapper call produced, the same
//! reasoning `platform-linux/tests/net_nonblocking.rs` gives for its
//! own raw `libc` calls.
#![cfg(windows)]
#![allow(unsafe_code)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::windows::io::AsRawSocket;

use platform::error::ErrorKind;
use platform::net::{TcpListener as _, TcpStream as _, UdpSocket as _};
use platform_windows::{WindowsTcpListener, WindowsTcpStream, WindowsUdpSocket};

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

#[test]
fn concrete_tcp_constructors_bypass_the_boxed_trait() {
    let listener = WindowsTcpListener::bind(loopback(0)).expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    assert_ne!(addr.port(), 0);

    let handle = std::thread::spawn(move || {
        let (mut stream, _peer) = listener.accept().expect("accept");
        let mut buf = [0u8; 4];
        let n = stream.read(&mut buf).expect("server read");
        assert_eq!(&buf[..n], b"ping");
    });

    let mut client = WindowsTcpStream::connect(addr).expect("connect");
    client.write(b"ping").expect("client write");
    drop(client);
    handle.join().expect("server thread");
}

#[test]
fn nonblocking_accept_with_no_pending_connection_returns_would_block_not_hang() {
    let listener = WindowsTcpListener::bind(loopback(0)).expect("bind");
    listener.set_nonblocking(true).expect("set_nonblocking");

    // No connection is pending — a blocking accept would hang forever;
    // this must return immediately with WouldBlock/WSAEWOULDBLOCK.
    let err = match listener.accept() {
        Ok(_) => panic!("no connection was pending"),
        Err(e) => e,
    };
    assert_eq!(err.kind, ErrorKind::WouldBlock);
}

#[test]
fn nonblocking_udp_recv_with_no_datagram_returns_would_block_not_hang() {
    let sock = WindowsUdpSocket::bind(loopback(0)).expect("bind");
    sock.set_nonblocking(true).expect("set_nonblocking");

    let mut buf = [0u8; 16];
    let err = sock.recv_from(&mut buf).expect_err("nothing sent yet");
    assert_eq!(err.kind, ErrorKind::WouldBlock);
}

/// `getsockopt(SOL_SOCKET, SO_TYPE, ...)` read directly via raw
/// `windows_sys` calls, bypassing this crate's own code — the same
/// "verify the actual OS-visible state, not just that our wrapper
/// returned a number" discipline the Linux suite's `fstat`-based
/// `as_raw_fd_reports_a_real_open_fd` test uses. Winsock has no
/// `fstat`-equivalent identity check; `SO_TYPE` is the closest
/// "the OS still recognizes this handle as a live socket of the right
/// kind" proof available.
#[test]
fn as_raw_socket_reports_a_real_socket_handle() {
    use windows_sys::Win32::Networking::WinSock::{getsockopt, SOCK_DGRAM, SOL_SOCKET, SO_TYPE};

    let sock = WindowsUdpSocket::bind(loopback(0)).expect("bind");
    let raw = sock.as_raw_socket();
    assert_ne!(raw, 0, "must be a real handle, not a null/sentinel value");

    let mut sock_type: i32 = 0;
    let mut len: i32 = std::mem::size_of::<i32>() as i32;
    // SAFETY: `raw` is a valid, open Winsock `SOCKET` owned by `sock`
    // for the duration of this call (cast to `usize`, `SOCKET`'s exact
    // representation — see `sys::net::OwnedSocket::raw`'s own doc);
    // `sock_type`/`len` are valid, exclusively borrowed out-params
    // `getsockopt` fills.
    let r = unsafe {
        getsockopt(
            raw as usize,
            SOL_SOCKET,
            SO_TYPE,
            (&mut sock_type as *mut i32).cast(),
            &mut len,
        )
    };
    assert_eq!(
        r, 0,
        "getsockopt(SO_TYPE) on the reported handle must succeed"
    );
    assert_eq!(sock_type, SOCK_DGRAM, "must be recognized as a UDP socket");
}
