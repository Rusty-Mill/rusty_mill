#![cfg(windows)]
// The Windows arm of `UnixListener`/`UnixStream` -- see `io/unix.rs`'s
// own docs and `docs/decision-request-windows-process-signal-ipc.md` for
// what's supported here versus the Unix arm (`tests/unix.rs`, which stays
// `#![cfg(unix)]`-only): no `UnixStream::pair` (no anonymous `AF_UNIX`
// pair primitive on Windows), no abstract-namespace addressing (Windows
// `AF_UNIX` is pathname-only), no `peer_cred` (no Windows peer-credential
// mechanism), no `FromRawSocket`/`IntoRawSocket` roundtrip (`platform_windows`
// has no owned-socket adoption yet). Everything else this file exercises
// -- `bind`/`accept`/`connect`/`connect_addr`/`bind_addr`/`read`/`write`/
// `local_addr`/`peer_addr`/`take_error`/`AsRawSocket` -- is real,
// reactor-driven `AF_UNIX` I/O over `platform_windows::{WindowsUnixListener,
// WindowsUnixStream}`, run for real on native Windows in this session (not
// just `cargo check --target x86_64-pc-windows-gnu`).

use rusty_tokio::io::{UnixListener, UnixSocketAddr, UnixStream};
use rusty_tokio::Runtime;
use std::os::windows::io::AsRawSocket;

fn temp_socket_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusty_tokio-test-{}-{}-{}.sock",
        std::process::id(),
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn unix_echo_roundtrip() {
    let rt = Runtime::new().unwrap();
    let path = temp_socket_path("echo");
    rt.block_on(async {
        let listener = UnixListener::bind(&path).unwrap();

        let server = rusty_tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            let mut buf = [0u8; 64];
            let n = stream.read(&mut buf).await.unwrap();
            stream.write_all(&buf[..n]).await.unwrap();
        });

        let client = UnixStream::connect(&path).await.unwrap();
        client.write_all(b"hello unix").await.unwrap();
        let mut buf = [0u8; 64];
        client.read_exact(&mut buf[..10]).await.unwrap();
        assert_eq!(&buf[..10], b"hello unix");

        server.await.unwrap();
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn bind_addr_then_connect_addr_round_trip_over_a_pathname() {
    let rt = Runtime::new().unwrap();
    let path = temp_socket_path("bind-addr-pathname");
    rt.block_on(async {
        let addr = UnixSocketAddr::from_pathname(&path).unwrap();
        assert_eq!(addr.as_pathname(), Some(path.as_path()));
        assert!(!addr.is_unnamed());

        let listener = UnixListener::bind_addr(&addr).unwrap();
        assert_eq!(
            listener.local_addr().unwrap().as_pathname(),
            Some(path.as_path())
        );

        let server = rusty_tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            let mut buf = [0u8; 5];
            stream.read(&mut buf).await.unwrap();
            stream.write_all(&buf).await.unwrap();
        });

        let addr = UnixSocketAddr::from_pathname(&path).unwrap();
        let client = UnixStream::connect_addr(&addr).await.unwrap();
        client.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 5];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");

        server.await.unwrap();
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn connect_addr_to_the_unnamed_address_fails_with_invalid_input() {
    let rt = Runtime::new().unwrap();
    let path = temp_socket_path("unnamed-reject");
    rt.block_on(async {
        // A connecting `AF_UNIX` stream socket that never itself `bind`s
        // stays unnamed -- the real, only way to obtain an unnamed
        // `UnixSocketAddr` on Windows (no abstract namespace, no public
        // "build one directly" constructor the way an unbound Unix-side
        // socket's `is_unnamed()` address can be inspected without a
        // live socket at all).
        let listener = UnixListener::bind(&path).unwrap();
        let server = rusty_tokio::spawn(async move { listener.accept().await.unwrap() });
        let client = UnixStream::connect(&path).await.unwrap();
        let _accepted = server.await.unwrap();

        let unnamed = client.local_addr().unwrap();
        assert!(unnamed.is_unnamed());
        assert!(unnamed.as_pathname().is_none());

        let err = UnixStream::connect_addr(&unnamed).await.err().unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        let err = UnixListener::bind_addr(&unnamed).err().unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn local_addr_and_peer_addr_report_the_bound_path() {
    let rt = Runtime::new().unwrap();
    let path = temp_socket_path("addrs");
    rt.block_on(async {
        let listener = UnixListener::bind(&path).unwrap();
        assert_eq!(
            listener.local_addr().unwrap().as_pathname(),
            Some(path.as_path())
        );

        let server = rusty_tokio::spawn(async move { listener.accept().await.unwrap() });
        let client = UnixStream::connect(&path).await.unwrap();
        let (accepted, _peer) = server.await.unwrap();

        assert_eq!(
            client.peer_addr().unwrap().as_pathname(),
            Some(path.as_path())
        );
        assert_eq!(
            accepted.local_addr().unwrap().as_pathname(),
            Some(path.as_path())
        );
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn unix_take_error_is_none_on_healthy_sockets() {
    let rt = Runtime::new().unwrap();
    let path = temp_socket_path("take-error");
    rt.block_on(async {
        let listener = UnixListener::bind(&path).unwrap();
        assert!(listener.take_error().unwrap().is_none());

        let server = rusty_tokio::spawn(async move { listener.accept().await.unwrap() });
        let client = UnixStream::connect(&path).await.unwrap();
        let (accepted, _peer) = server.await.unwrap();

        assert!(client.take_error().unwrap().is_none());
        assert!(accepted.take_error().unwrap().is_none());
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn many_concurrent_unix_connections() {
    let rt = Runtime::builder().worker_threads(4).build().unwrap();
    let path = temp_socket_path("many");
    rt.block_on(async {
        let listener = UnixListener::bind(&path).unwrap();

        let server = rusty_tokio::spawn(async move {
            for _ in 0..50 {
                let (stream, _peer) = listener.accept().await.unwrap();
                rusty_tokio::spawn(async move {
                    let mut buf = [0u8; 8];
                    let n = stream.read(&mut buf).await.unwrap();
                    stream.write_all(&buf[..n]).await.unwrap();
                });
            }
        });

        let mut clients = Vec::new();
        for i in 0..50u8 {
            let path = path.clone();
            clients.push(rusty_tokio::spawn(async move {
                let stream = UnixStream::connect(&path).await.unwrap();
                stream.write_all(&[i]).await.unwrap();
                let mut buf = [0u8; 1];
                stream.read_exact(&mut buf).await.unwrap();
                assert_eq!(buf[0], i);
            }));
        }
        for c in clients {
            c.await.unwrap();
        }
        server.await.unwrap();
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn as_raw_socket_returns_a_distinct_handle_per_stream() {
    let rt = Runtime::new().unwrap();
    let path = temp_socket_path("as-raw-socket");
    rt.block_on(async {
        let listener = UnixListener::bind(&path).unwrap();
        assert_ne!(listener.as_raw_socket(), 0);

        let server = rusty_tokio::spawn(async move { listener.accept().await.unwrap() });
        let client = UnixStream::connect(&path).await.unwrap();
        let (accepted, _peer) = server.await.unwrap();

        assert_ne!(client.as_raw_socket(), 0);
        assert_ne!(accepted.as_raw_socket(), 0);
        assert_ne!(client.as_raw_socket(), accepted.as_raw_socket());
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_live_listener_rejects_a_second_bind_at_the_same_path() {
    let rt = Runtime::new().unwrap();
    let path = temp_socket_path("double-bind");
    rt.block_on(async {
        let _listener = UnixListener::bind(&path).unwrap();
        let second = UnixListener::bind(&path);
        assert!(second.is_err());
    });
    let _ = std::fs::remove_file(&path);
}
