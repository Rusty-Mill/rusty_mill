//! Deadlines, cancellation, shutdown and a handler that panics.
#![allow(clippy::unwrap_used)]

mod client_common;
mod server_common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use client_common::*;
use rusty_prost::Message;
use rusty_tokio::sync::mpsc;
use rusty_tokio::time::{sleep, timeout};
use rusty_tonic::client::{Request, Response};
use rusty_tonic::server::{Incoming, Router, Sink};
use rusty_tonic::{Code, RequestHead};
use server_common::*;

const PATH: &str = "/pkg.Svc/Method";

/// Sets a flag when dropped: proof that a handler's future was dropped.
struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn headers_with_timeout(t: Duration) -> Vec<(String, Vec<u8>)> {
    let mut head = RequestHead::new(PATH, "localhost");
    head.timeout = Some(t);
    head.to_headers()
}

fn one_message() -> Vec<Vec<u8>> {
    vec![frame(&blob(1, 1).encode_to_vec().unwrap())]
}

async fn next<T>(rx: &mut mpsc::Receiver<T>) -> T {
    timeout(FIVE, rx.recv()).await.unwrap().unwrap()
}

// ---- deadlines ------------------------------------------------------------------------------

#[rusty_tokio::test]
async fn a_handler_past_the_clients_deadline_is_dropped_and_the_call_is_deadline_exceeded() {
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = dropped.clone();
    let router = Router::new().unary(PATH, move |r: Request<Blob>| {
        let guard = Dropped(flag.clone());
        async move {
            let _guard = guard;
            sleep(FIVE * 2).await;
            Ok(Response::new(r.message))
        }
    });
    let server = serve(router);
    let started = std::time::Instant::now();
    let r = raw(
        server.local_addr(),
        headers_with_timeout(SHORT),
        &one_message(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(r.status().code(), Code::DeadlineExceeded);
    assert!(r.trailers_only(), "nothing had been sent");
    assert!(started.elapsed() >= SHORT && started.elapsed() < FIVE);
    assert!(dropped.load(Ordering::SeqCst), "the handler was dropped");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_stream_past_the_deadline_ends_with_the_status_after_what_was_sent() {
    let router =
        Router::new().server_streaming(PATH, |_r: Request<Blob>, mut out: Sink<Blob>| async move {
            out.send(&blob(1, 1)).await?;
            sleep(FIVE * 2).await;
            Ok(())
        });
    let server = serve(router);
    let r = raw(
        server.local_addr(),
        headers_with_timeout(SHORT),
        &one_message(),
        true,
    )
    .await
    .unwrap();
    assert!(!r.data.is_empty(), "the first reply arrived");
    assert_eq!(r.status().code(), Code::DeadlineExceeded);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_call_without_a_deadline_is_not_cut_off() {
    let router = Router::new().unary(PATH, |r: Request<Blob>| async move {
        sleep(SHORT * 2).await;
        Ok(Response::new(r.message))
    });
    let server = serve(router);
    let reply = channel(&server)
        .unary::<Blob, Blob>(PATH, Request::new(blob(3, 3)))
        .await
        .unwrap();
    assert_eq!(reply.message.n, 3);
    server.shutdown().await;
}

// ---- cancellation ---------------------------------------------------------------------------

#[rusty_tokio::test]
async fn a_client_that_cancels_makes_the_handlers_next_send_fail_cancelled() {
    let (tx, mut rx) = mpsc::channel(4);
    let router =
        Router::new().server_streaming(PATH, move |_r: Request<Blob>, mut out: Sink<Blob>| {
            let tx = tx.clone();
            async move {
                loop {
                    if let Err(e) = out.send(&blob(1, 1000)).await {
                        tx.send(e.code()).await.unwrap();
                        return Err(e);
                    }
                    sleep(Duration::from_millis(10)).await;
                }
            }
        });
    let server = serve(router);
    let mut stream = channel(&server)
        .server_streaming::<Blob, Blob>(PATH, Request::new(blob(0, 0)))
        .await
        .unwrap();
    assert!(stream.message().await.unwrap().is_some());
    drop(stream);
    assert_eq!(next(&mut rx).await, Code::Cancelled);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_client_that_cancels_makes_the_handlers_read_fail_cancelled() {
    let (tx, mut rx) = mpsc::channel(4);
    let router =
        Router::new().bidirectional(PATH, move |r: Request<Incoming<Blob>>, _out: Sink<Blob>| {
            let tx = tx.clone();
            async move {
                let mut incoming = r.message;
                loop {
                    match incoming.message().await {
                        Ok(Some(_)) => tx.send(None).await.unwrap(),
                        Ok(None) => return Ok(()),
                        Err(e) => {
                            tx.send(Some(e.code())).await.unwrap();
                            return Err(e);
                        }
                    }
                }
            }
        });
    let server = serve(router);
    let (mut sender, stream) = channel(&server)
        .bidirectional::<Blob, Blob>(PATH, Request::new(()))
        .await
        .unwrap();
    sender.send(&blob(1, 1)).await.unwrap();
    assert_eq!(next(&mut rx).await, None, "the handler read the message");
    drop(stream);
    drop(sender);
    assert_eq!(next(&mut rx).await, Some(Code::Cancelled));
    server.shutdown().await;
}

// ---- shutdown, load, panics -----------------------------------------------------------------

#[rusty_tokio::test]
async fn shutdown_lets_the_call_in_flight_finish() {
    let (tx, mut rx) = mpsc::channel(1);
    let router = Router::new().unary(PATH, move |r: Request<Blob>| {
        let tx = tx.clone();
        async move {
            tx.send(()).await.unwrap();
            sleep(SHORT * 2).await;
            Ok(Response::new(r.message))
        }
    });
    let server = serve(router);
    let c = channel(&server);
    let call =
        rusty_tokio::spawn(
            async move { c.unary::<Blob, Blob>(PATH, Request::new(blob(9, 9))).await },
        );
    next(&mut rx).await;
    server.shutdown().await;
    assert_eq!(call.await.unwrap().unwrap().message.n, 9);
}

#[rusty_tokio::test]
async fn many_calls_at_once_each_get_their_own_reply() {
    let router = Router::new().unary(PATH, |r: Request<Blob>| async move {
        sleep(Duration::from_millis(20)).await;
        Ok(Response::new(r.message))
    });
    let server = serve(router);
    let c = channel(&server);
    let calls: Vec<_> = (0..64u32)
        .map(|n| {
            let c = c.clone();
            rusty_tokio::spawn(async move {
                c.unary::<Blob, Blob>(PATH, Request::new(blob(n, 2000)))
                    .await
                    .map(|r| r.message)
            })
        })
        .collect();
    for (n, call) in calls.into_iter().enumerate() {
        assert_eq!(call.await.unwrap().unwrap(), blob(n as u32, 2000));
    }
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_handler_that_panics_fails_its_call_and_the_server_goes_on() {
    let router = Router::new()
        .unary(PATH, |_r: Request<Blob>| async move {
            if true {
                panic!("a handler bug");
            }
            Ok(Response::new(blob(0, 0)))
        })
        .unary("/pkg.Svc/Fine", |r: Request<Blob>| async move {
            Ok(Response::new(r.message))
        });
    let server = serve(router);
    let c = channel(&server);
    let err = c
        .unary::<Blob, Blob>(PATH, Request::new(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Internal);
    let ok = c
        .unary::<Blob, Blob>("/pkg.Svc/Fine", Request::new(blob(2, 2)))
        .await
        .unwrap();
    assert_eq!(ok.message.n, 2);
    server.shutdown().await;
}
