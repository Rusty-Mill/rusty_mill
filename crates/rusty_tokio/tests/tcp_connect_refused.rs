//! `TcpStream::connect`/`TcpSocket::connect` against a port nothing is
//! listening on must come back with an error -- promptly, and from
//! `connect` itself, not from the first read or write on a stream that
//! was handed back as if it had connected.
//!
//! Regression coverage for Rusty-Mill/rusty_mill#137: a non-blocking
//! connect that returns in-progress used to be registered with the
//! reactor's optimistic "already writable" default, so `connect`'s
//! `SO_ERROR` check ran before the handshake had resolved, saw nothing,
//! and returned a stream that was still in `SYN_SENT`. Linux hid it: a
//! loopback `connect(2)` reports `EINPROGRESS` but has already processed
//! the handshake (or the RST) inside the call, so the premature check
//! happened to see a settled `SO_ERROR`. Windows, where the result is
//! genuinely still pending when `connect` returns, hung on a refused
//! loopback connect until the surrounding test runner killed it. These
//! tests run on every CI platform; the Windows leg is the one where the
//! wait is real, and the Linux leg proves the write-pending registration
//! still observes a connect that lands before the reactor looks.

use rusty_tokio::io::{TcpSocket, TcpStream};
use rusty_tokio::time::timeout;
use std::net::{SocketAddr, TcpListener};
use std::time::Duration;

/// Well past any OS's connect-refused reporting delay (Windows retries
/// the SYN a couple of times before giving up, ~1-2s), but far short of
/// the 600s test-runner kill that the original bug hit.
const CONNECT_DEADLINE: Duration = Duration::from_secs(30);

/// An ephemeral loopback port that was bound a moment ago and is now
/// closed -- connecting to it gets refused (nothing listens there), and
/// the kernel just gave it out so it's not in use by anything else.
fn refused_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);
    addr
}

fn assert_refused(result: Result<std::io::Result<TcpStream>, rusty_tokio::time::Elapsed>) {
    match result {
        Ok(Err(err)) => {
            // `ConnectionRefused` everywhere this crate runs; the point
            // of the test is that an error surfaced from `connect` at
            // all, so anything else is reported rather than asserted.
            assert_eq!(
                err.kind(),
                std::io::ErrorKind::ConnectionRefused,
                "connect failed, but not with ConnectionRefused: {err:?}"
            );
        }
        Ok(Ok(_stream)) => panic!("connect to a closed port reported success"),
        Err(_elapsed) => panic!(
            "connect to a closed port neither succeeded nor failed within {CONNECT_DEADLINE:?}"
        ),
    }
}

#[rusty_tokio::test]
async fn tcp_stream_connect_to_a_closed_loopback_port_is_refused() {
    let addr = refused_addr();
    let result = timeout(CONNECT_DEADLINE, TcpStream::connect_addr(addr)).await;
    assert_refused(result);
}

#[rusty_tokio::test]
async fn tcp_stream_connect_resolves_and_is_refused_too() {
    // Through the `ToSocketAddrs` path (`&str` -> resolution -> each
    // candidate tried in turn) rather than `connect_addr` directly.
    let addr = refused_addr();
    let result = timeout(CONNECT_DEADLINE, TcpStream::connect(addr.to_string())).await;
    assert_refused(result);
}

#[rusty_tokio::test]
async fn tcp_socket_connect_to_a_closed_loopback_port_is_refused() {
    let addr = refused_addr();
    let socket = TcpSocket::new_v4().expect("TcpSocket::new_v4");
    let result = timeout(CONNECT_DEADLINE, socket.connect(addr)).await;
    assert_refused(result);
}

/// The exact address the report in #137 used: a low, privileged port
/// nothing binds in CI. Same expectation as the ephemeral-port cases.
#[rusty_tokio::test]
async fn connect_to_port_one_on_loopback_is_refused() {
    let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
    let result = timeout(CONNECT_DEADLINE, TcpStream::connect_addr(addr)).await;
    assert_refused(result);
}

/// The other side of the coin: a connect that *does* succeed still
/// does, now that an in-flight connect starts write-pending -- the
/// backend must report the socket writable once the handshake lands.
#[rusty_tokio::test]
async fn tcp_stream_connect_to_a_listening_port_still_succeeds() {
    let listener = rusty_tokio::io::TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = rusty_tokio::spawn(async move { listener.accept().await.map(|_| ()) });
    let stream = timeout(CONNECT_DEADLINE, TcpStream::connect_addr(addr))
        .await
        .expect("connect neither succeeded nor failed in time")
        .expect("connect to a listening port failed");
    assert_eq!(stream.peer_addr().unwrap(), addr);
    accept.await.unwrap().unwrap();
}
