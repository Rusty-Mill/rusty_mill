//! Timeouts, keepalive, the connection cap and graceful shutdown.
#![allow(clippy::unwrap_used)]

mod common;

use std::io::{Read, Write};
use std::net::TcpListener as StdListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use rusty_hyper::{connect, ConnectOptions, Error, Keepalive, Request, Respond, ServeOptions};
use rusty_tokio::io::TcpStream;
use rusty_tokio::sync::oneshot;
use rusty_tokio::time::timeout;

const FIVE: Duration = Duration::from_secs(5);

/// A peer that speaks just enough HTTP/2 to be a server: it sends SETTINGS,
/// acknowledges the client's, answers the first `answer_pings` PINGs and
/// ignores the rest.
fn raw_peer(answer_pings: usize) -> std::net::SocketAddr {
    let l = StdListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut preface = [0u8; 24];
        s.read_exact(&mut preface).unwrap();
        // An empty SETTINGS frame: length 0, type 4, flags 0, stream 0.
        s.write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0]).unwrap();
        let mut answered = 0;
        loop {
            let mut head = [0u8; 9];
            if s.read_exact(&mut head).is_err() {
                return;
            }
            let len = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
            let mut payload = vec![0u8; len];
            if s.read_exact(&mut payload).is_err() {
                return;
            }
            match (head[3], head[4] & 1) {
                (4, 0) => {
                    // The client's SETTINGS: acknowledge.
                    let _ = s.write_all(&[0, 0, 0, 4, 1, 0, 0, 0, 0]);
                }
                (6, 0) if answered < answer_pings => {
                    answered += 1;
                    let mut ack = vec![0, 0, 8, 6, 1, 0, 0, 0, 0];
                    ack.extend_from_slice(&payload);
                    let _ = s.write_all(&ack);
                }
                _ => {}
            }
        }
    });
    addr
}

#[rusty_tokio::test]
async fn a_peer_that_never_speaks_http2_times_out_connecting() {
    let l = StdListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let held = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        std::thread::sleep(Duration::from_secs(2));
        drop(s);
    });
    let o = ConnectOptions {
        connect_timeout: Duration::from_millis(200),
        ..ConnectOptions::default()
    };
    let r = connect(&format!("http://{addr}"), o).await;
    assert!(matches!(r, Err(Error::Timeout(_))), "{:?}", r.err());
    held.join().unwrap();
}

#[rusty_tokio::test]
async fn a_client_that_never_finishes_the_handshake_is_dropped_by_the_server() {
    let pki = pki();
    let server = start(
        listener(),
        Some(pki.acceptor(&[b"h2"])),
        ServeOptions {
            handshake_timeout: Duration::from_millis(200),
            ..ServeOptions::default()
        },
        echo,
    );
    // Connects and says nothing.
    let idle = TcpStream::connect(server.local_addr()).await.unwrap();
    let mut buf = [0u8; 16];
    let n = timeout(FIVE, idle.read(&mut buf))
        .await
        .expect("the server must give up on a silent client")
        .unwrap_or(0);
    assert_eq!(n, 0, "the server closes the connection");
    // And it still serves others.
    let o = ConnectOptions {
        trust: pki.trust(),
        ..ConnectOptions::default()
    };
    let conn = connect(&url(&server, "https"), o).await.unwrap();
    call(conn.client(), "https", &[], b"ok".to_vec())
        .await
        .unwrap();
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_plain_client_that_stays_silent_is_dropped_by_the_server() {
    let server = start(
        listener(),
        None,
        ServeOptions {
            handshake_timeout: Duration::from_millis(200),
            ..ServeOptions::default()
        },
        echo,
    );
    let idle = TcpStream::connect(server.local_addr()).await.unwrap();
    let mut buf = [0u8; 16];
    let n = timeout(FIVE, idle.read(&mut buf))
        .await
        .expect("the server must give up on a silent client")
        .unwrap_or(0);
    assert_eq!(n, 0, "the server closes the connection");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn keepalive_closes_a_connection_whose_pings_go_unanswered() {
    // The first PING (the connect check) is answered, the keepalive's are not.
    let addr = raw_peer(1);
    let o = ConnectOptions {
        keepalive: Some(Keepalive {
            interval: Duration::from_millis(50),
            timeout: Duration::from_millis(150),
        }),
        ..ConnectOptions::default()
    };
    let mut conn = connect(&format!("http://{addr}"), o).await.unwrap();
    let ended = timeout(FIVE, conn.closed())
        .await
        .expect("keepalive must end the connection");
    assert!(ended.is_err());
    assert!(call(conn.client(), "http", &[], b"x".to_vec())
        .await
        .is_err());
}

#[rusty_tokio::test]
async fn keepalive_leaves_a_healthy_connection_alone() {
    let server = start(listener(), None, ServeOptions::default(), echo);
    let o = ConnectOptions {
        keepalive: Some(Keepalive {
            interval: Duration::from_millis(30),
            timeout: Duration::from_secs(2),
        }),
        ..ConnectOptions::default()
    };
    let conn = connect(&url(&server, "http"), o).await.unwrap();
    rusty_tokio::time::sleep(Duration::from_millis(400)).await;
    let r = call(conn.client(), "http", &[], b"still here".to_vec())
        .await
        .unwrap();
    assert_eq!(r.body, b"still here");
    conn.close().await.unwrap();
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn the_connection_cap_refuses_the_next_and_frees_a_slot_when_one_closes() {
    let server = start(
        listener(),
        None,
        ServeOptions {
            max_connections: 1,
            ..ServeOptions::default()
        },
        echo,
    );
    let u = url(&server, "http");
    let first = connect(&u, ConnectOptions::default()).await.unwrap();
    call(first.client(), "http", &[], b"1".to_vec())
        .await
        .unwrap();

    let second = timeout(FIVE, connect(&u, ConnectOptions::default()))
        .await
        .expect("a refused connection fails promptly");
    assert!(second.is_err(), "the cap of 1 refuses a second connection");

    first.close().await.unwrap();
    // The slot is released when the server sees the first connection end.
    let mut third = None;
    for _ in 0..100 {
        if let Ok(c) = connect(&u, ConnectOptions::default()).await {
            third = Some(c);
            break;
        }
        rusty_tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let third = third.expect("a slot frees once the first connection is gone");
    call(third.client(), "http", &[], b"3".to_vec())
        .await
        .unwrap();
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn shutdown_lets_a_call_in_flight_finish_and_refuses_new_connections() {
    // The handler holds its call open until released.
    let (release, held) = oneshot::channel::<()>();
    let held = Arc::new(Mutex::new(Some(held)));
    let (started_tx, started) = oneshot::channel::<()>();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let handler = move |req: Request, respond: Respond| {
        let held = held.clone();
        let started_tx = started_tx.clone();
        async move {
            if let Some(tx) = started_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
            let rx = held.lock().unwrap().take();
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            echo(req, respond).await;
        }
    };
    let server = start(listener(), None, ServeOptions::default(), handler);
    let addr = server.local_addr();
    let conn = connect(&format!("http://{addr}"), ConnectOptions::default())
        .await
        .unwrap();
    let h = conn.client().clone();
    let inflight =
        rusty_tokio::spawn(
            async move { call(&h, "http", &[], b"slow".to_vec()).await.unwrap().body },
        );
    started.await.unwrap();

    let mut stopping = rusty_tokio::spawn(server.shutdown());
    // Once the server has begun shutting down it takes no new connections.
    let mut refused = false;
    for _ in 0..100 {
        if connect(&format!("http://{addr}"), ConnectOptions::default())
            .await
            .is_err()
        {
            refused = true;
            break;
        }
        rusty_tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(refused, "no new connection once shutdown has begun");
    assert!(
        timeout(Duration::from_millis(200), &mut stopping)
            .await
            .is_err(),
        "shutdown waits for the call in flight"
    );

    release.send(()).unwrap();
    let body = timeout(FIVE, inflight).await.unwrap().unwrap();
    assert_eq!(body, b"slow", "the call in flight finishes");
    timeout(FIVE, stopping).await.unwrap().unwrap();
}
