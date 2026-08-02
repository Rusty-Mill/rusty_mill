//! Net parity suite (rustils#48), textually mirroring
//! `platform-linux/tests/net_parity.rs` and `platform-windows/tests/
//! net_parity.rs` — the recorded follow-up (both those files' own doc
//! comments) is extracting this into a shared crate once a third
//! backend would otherwise mean a third copy. This *is* that third
//! copy, deliberately kept identical rather than factored out now,
//! matching the existing precedent.
//!
//! The `bsd` module's tests run for real on three CI legs — macOS
//! (rustils#48), and FreeBSD and OpenBSD in a VM (rustils#86). They live
//! inside one `#[cfg(any(...))]`-gated module, referencing
//! `platform_bsd::BsdNet` by full path rather than a top-level `use`, so
//! the file still compiles (mock tests only) on Linux and Windows hosts
//! — the same discipline `net_parity.rs`'s Linux/Windows copies already
//! use for their own OS-specific types.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use platform::error::ErrorKind;
use platform::net::Net;

/// TCP's assertion set.
fn assert_net_behavior(net: &dyn Net) {
    let listener = net
        .tcp_listen(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .expect("listen");
    let addr = listener.local_addr().expect("local_addr");
    assert_ne!(addr.port(), 0, "an ephemeral port was assigned");

    let handle = std::thread::spawn(move || {
        let (mut stream, peer) = listener.accept().expect("accept");
        assert_eq!(peer.ip(), addr.ip());
        let mut buf = [0u8; 16];
        let n = stream.read(&mut buf).expect("server read");
        assert_eq!(&buf[..n], b"ping");
        stream.write(b"pong").expect("server write");
    });

    let mut client = net.tcp_connect(addr).expect("connect");
    assert_eq!(client.peer_addr().expect("peer_addr"), addr);
    let local = client.local_addr().expect("local_addr");
    assert_eq!(local.ip(), addr.ip());
    assert_ne!(local.port(), 0);

    client
        .set_nodelay(true)
        .expect("set_nodelay must not error");

    client.write(b"ping").expect("client write");
    let mut buf = [0u8; 16];
    let n = client.read(&mut buf).expect("client read");
    assert_eq!(&buf[..n], b"pong");
    handle.join().expect("server thread panicked");
    drop(client);

    // Nothing is listening at this port once the listener above drops
    // — connecting must fail, not hang or silently succeed.
    let refused_addr = SocketAddr::new(addr.ip(), addr.port());
    let e = net
        .tcp_connect(refused_addr)
        .err()
        .expect("must refuse: nothing listening");
    assert_eq!(e.kind, ErrorKind::ConnectionRefused);

    // set_read_timeout: a peer that stays connected but sends nothing
    // must make a short-timeout read return promptly, not hang until
    // the peer eventually does something.
    let listener2 = net
        .tcp_listen(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .expect("listen (timeout case)");
    let addr2 = listener2.local_addr().expect("local_addr");
    let handle2 = std::thread::spawn(move || {
        let (stream, _peer) = listener2.accept().expect("accept (timeout case)");
        std::thread::sleep(Duration::from_millis(500));
        drop(stream);
    });
    let mut timeout_client = net.tcp_connect(addr2).expect("connect (timeout case)");
    timeout_client
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set_read_timeout");
    let started = Instant::now();
    let mut buf = [0u8; 16];
    let e = timeout_client
        .read(&mut buf)
        .expect_err("must time out: the peer never sends anything");
    assert!(
        matches!(e.kind, ErrorKind::WouldBlock | ErrorKind::TimedOut),
        "unexpected kind for an expired read timeout: {:?}",
        e.kind
    );
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "read timeout took far longer than the 100ms requested"
    );
    handle2.join().expect("server thread panicked");
}

/// Unix domain sockets' assertion set.
fn assert_unix_behavior(net: &dyn Net, label: &str) {
    let path = std::env::temp_dir().join(format!(
        "rustils-net-parity-{label}-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);

    let listener = net.unix_listen(&path).expect("listen");
    let local = listener.local_addr().expect("local_addr");
    assert_eq!(local.as_deref(), Some(path.as_path()));

    let handle = std::thread::spawn(move || {
        let (mut stream, peer) = listener.accept().expect("accept");
        assert_eq!(peer, None, "an unbound connecting client has no peer path");
        let mut buf = [0u8; 16];
        let n = stream.read(&mut buf).expect("server read");
        assert_eq!(&buf[..n], b"ping");
        stream.write(b"pong").expect("server write");
    });

    let mut client = net.unix_connect(&path).expect("connect");
    assert_eq!(
        client.peer_addr().expect("peer_addr").as_deref(),
        Some(path.as_path())
    );
    assert_eq!(client.local_addr().expect("local_addr"), None);

    client.write(b"ping").expect("client write");
    let mut buf = [0u8; 16];
    let n = client.read(&mut buf).expect("client read");
    assert_eq!(&buf[..n], b"pong");
    handle.join().expect("server thread panicked");
    drop(client);

    let e = net
        .unix_connect(&path)
        .err()
        .expect("must refuse: nothing listening");
    assert_eq!(e.kind, ErrorKind::ConnectionRefused);

    // Stale-cleanup bind: the path above still names a real (if now
    // stale) socket file — dropping a listener never unlinks it. A
    // fresh `unix_listen` on the same path must reclaim it rather than
    // fail `AddrInUse`.
    let listener2 = net
        .unix_listen(&path)
        .expect("stale-cleanup bind must reclaim the leftover path");
    drop(listener2);

    let _ = std::fs::remove_file(&path);
}

/// UDP's own assertion set.
fn assert_udp_behavior(net: &dyn Net) {
    let server = net
        .udp_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .expect("bind");
    let server_addr = server.local_addr().expect("local_addr");
    assert_ne!(server_addr.port(), 0, "an ephemeral port was assigned");

    let client = net
        .udp_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .expect("bind");
    let client_addr = client.local_addr().expect("local_addr");

    client.send_to(b"ping", server_addr).expect("send_to");
    let mut buf = [0u8; 16];
    let (n, peer) = server.recv_from(&mut buf).expect("recv_from");
    assert_eq!(&buf[..n], b"ping");
    assert_eq!(peer.ip(), client_addr.ip());
    assert_eq!(peer.port(), client_addr.port());

    server.send_to(b"pong", client_addr).expect("send_to");
    let (n, peer) = client.recv_from(&mut buf).expect("recv_from");
    assert_eq!(&buf[..n], b"pong");
    assert_eq!(peer.ip(), server_addr.ip());
    assert_eq!(peer.port(), server_addr.port());

    // Fire-and-forget: sending to an address nothing is bound to must
    // not error.
    let nobody = net
        .udp_bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .expect("bind");
    let nobody_addr = nobody.local_addr().expect("local_addr");
    drop(nobody);
    client
        .send_to(b"into the void", nobody_addr)
        .expect("send_to must not fail just because nothing is bound there");
}

#[test]
fn mock_net_conforms() {
    assert_net_behavior(&platform_mock::MockNet);
}

#[test]
fn mock_unix_conforms() {
    assert_unix_behavior(&platform_mock::MockNet, "mock");
}

#[test]
fn mock_udp_conforms() {
    assert_udp_behavior(&platform_mock::MockNet);
}

/// The real-backend half of the suite. One `cfg` on the module rather
/// than one per test: the gate has five arms now (rustils#86) and
/// repeating it three times is three chances for the copies to drift
/// apart. Must stay textually identical to the crate's own `lib.rs`
/// gate — a test that silently stops compiling on a target the crate
/// still claims is exactly the regression this suite exists to catch.
#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
mod bsd {
    use super::{assert_net_behavior, assert_udp_behavior, assert_unix_behavior};

    #[test]
    fn bsd_net_conforms() {
        assert_net_behavior(&platform_bsd::BsdNet);
    }

    #[test]
    fn bsd_unix_conforms() {
        assert_unix_behavior(&platform_bsd::BsdNet, "bsd");
    }

    #[test]
    fn bsd_udp_conforms() {
        assert_udp_behavior(&platform_bsd::BsdNet);
    }
}
