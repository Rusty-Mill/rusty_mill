//! Deadlines, cancellation, connection reuse and reconnection of the client.
#![allow(clippy::unwrap_used)]

mod client_common;

use std::time::Duration;

use client_common::*;
use rusty_h2::server::builder::Builder;
use rusty_hyper::{serve, Request, Respond, ServeOptions};
use rusty_tokio::sync::{mpsc, oneshot};
use rusty_tokio::time::{sleep, timeout};
use rusty_tonic::client::{Channel, ChannelOptions};
use rusty_tonic::{Code, FrameDecoder, MetadataMap, Status};

const PATH: &str = "/pkg.Svc/Method";

fn request(blob: Blob) -> rusty_tonic::client::Request<Blob> {
    rusty_tonic::client::Request::new(blob)
}

fn url(addr: std::net::SocketAddr) -> String {
    format!("http://{addr}")
}

/// A server that reports the `grpc-timeout` it was sent, then answers.
#[rusty_tokio::test]
async fn the_shorter_of_the_call_and_channel_deadlines_is_sent() {
    let (tx, mut rx) = mpsc::channel(8);
    let server = start(move |mut req: Request, respond: Respond| {
        let tx = tx.clone();
        async move {
            let call = read_call(&mut req).await;
            tx.send(call.head.timeout).await.unwrap();
            reply(respond, &call.blobs(), &Status::ok()).await;
        }
    });
    let options = ChannelOptions {
        timeout: Some(Duration::from_secs(30)),
        ..ChannelOptions::default()
    };
    let channel = Channel::new(&url(server.local_addr()), options).unwrap();

    // The channel's, when the call has none.
    channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap();
    let sent = rx.recv().await.unwrap().unwrap();
    assert!(sent > Duration::from_secs(25) && sent <= Duration::from_secs(30));

    // The call's, when shorter.
    channel
        .unary::<Blob, Blob>(
            PATH,
            request(blob(1, 1)).with_timeout(Duration::from_secs(5)),
        )
        .await
        .unwrap();
    let sent = rx.recv().await.unwrap().unwrap();
    assert!(sent > Duration::from_secs(3) && sent <= Duration::from_secs(5));

    // The channel's, when the call's is longer.
    channel
        .unary::<Blob, Blob>(
            PATH,
            request(blob(1, 1)).with_timeout(Duration::from_secs(90)),
        )
        .await
        .unwrap();
    let sent = rx.recv().await.unwrap().unwrap();
    assert!(sent <= Duration::from_secs(30));

    // And none at all when neither has one.
    let plain = channel_without_timeout(&server);
    plain
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap();
    assert_eq!(rx.recv().await.unwrap(), None);
}

fn channel_without_timeout(server: &rusty_hyper::Server) -> Channel {
    channel(server)
}

/// The next report from the server, which fails the test rather than hanging it.
async fn next(rx: &mut mpsc::Receiver<String>) -> String {
    timeout(FIVE, rx.recv()).await.unwrap().unwrap()
}

/// Reads the request's first message, reports "started", and then waits, reporting how the call
/// ended: the stream being reset, noticed either on the request body or by failing to write.
async fn stall(mut req: Request, respond: Respond, tx: mpsc::Sender<String>) {
    let mut send = start_response(respond, &MetadataMap::new()).await;
    let mut decoder = FrameDecoder::new(usize::MAX);
    let _ = next_message(&mut req.body, &mut decoder).await;
    tx.send("started".to_string()).await.unwrap();
    let mut body_open = true;
    let ended = loop {
        if body_open {
            match timeout(Duration::from_millis(20), req.body.next()).await {
                Ok(Some(Err(e))) => break format!("{e}"),
                Ok(None) => body_open = false,
                Ok(Some(Ok(_))) | Err(_) => {}
            }
        } else {
            sleep(Duration::from_millis(20)).await;
        }
        if let Err(e) = send.send_data(Vec::new(), false).await {
            break format!("{e}");
        }
    };
    tx.send(ended).await.unwrap();
}

/// A server whose every call stalls (see [`stall`]).
fn stalling_server() -> (rusty_hyper::Server, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel(8);
    let server = start(move |req: Request, respond: Respond| stall(req, respond, tx.clone()));
    (server, rx)
}

#[rusty_tokio::test]
async fn a_unary_call_past_its_deadline_is_deadline_exceeded_and_cancels_the_stream() {
    let (server, mut ended) = stalling_server();
    let channel = channel(&server);
    // Long enough for the server to have started the call, even on a loaded machine.
    let limit = Duration::from_millis(600);
    let started = std::time::Instant::now();
    let err = channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)).with_timeout(limit))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);
    assert!(started.elapsed() >= limit && started.elapsed() < FIVE * 2);
    assert_eq!(next(&mut ended).await, "started");
    // The request was whole when the stream was reset, so the server learns of it by failing
    // to write, as a closed stream.
    let how = next(&mut ended).await;
    assert!(how.contains("reset") || how.contains("closed"), "{how}");
}

#[rusty_tokio::test]
async fn a_channel_deadline_applies_to_calls_without_their_own() {
    let (server, _ended) = stalling_server();
    let options = ChannelOptions {
        timeout: Some(SHORT),
        ..ChannelOptions::default()
    };
    let channel = Channel::new(&url(server.local_addr()), options).unwrap();
    let err = channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);
}

#[rusty_tokio::test]
async fn a_stream_that_stalls_past_the_deadline_fails_after_what_arrived() {
    let server = start(|mut req: Request, respond: Respond| async move {
        let _ = read_call(&mut req).await;
        let mut send = start_response(respond, &MetadataMap::new()).await;
        send_message(&mut send, &blob(1, 3)).await;
        sleep(FIVE).await;
        let _ = send.send_data(Vec::new(), true).await;
    });
    let channel = channel(&server);
    // Long enough for the first reply to arrive even on a loaded machine.
    let limit = Duration::from_millis(600);
    let mut stream = channel
        .server_streaming::<Blob, Blob>(PATH, request(blob(0, 0)).with_timeout(limit))
        .await
        .unwrap();
    assert_eq!(stream.message().await.unwrap(), Some(blob(1, 3)));
    let err = stream.message().await.unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);
    // And again, the same.
    assert_eq!(
        stream.message().await.unwrap_err().code(),
        Code::DeadlineExceeded
    );
}

#[rusty_tokio::test]
async fn a_call_that_never_gets_its_response_headers_honours_the_deadline_too() {
    let server = start(|mut req: Request, respond: Respond| async move {
        let _ = read_call(&mut req).await;
        sleep(FIVE).await;
        drop(respond);
    });
    let channel = channel(&server);
    let err = channel
        .server_streaming::<Blob, Blob>(PATH, request(blob(0, 0)).with_timeout(SHORT))
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), Code::DeadlineExceeded, "{err:?}");
}

#[rusty_tokio::test]
async fn dropping_a_stream_cancels_the_call() {
    let (server, mut ended) = stalling_server();
    let channel = channel(&server);
    let (mut sender, stream) = channel
        .bidirectional::<Blob, Blob>(PATH, rusty_tonic::client::Request::new(()))
        .await
        .unwrap();
    sender.send(&blob(1, 1)).await.unwrap();
    assert_eq!(next(&mut ended).await, "started");
    drop(stream);
    drop(sender);
    let how = next(&mut ended).await;
    assert!(how.contains("reset"), "the server saw {how}");
}

#[rusty_tokio::test]
async fn dropping_a_server_stream_cancels_the_call() {
    let (server, mut ended) = stalling_server();
    let mut stream = channel(&server)
        .server_streaming::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap();
    assert_eq!(next(&mut ended).await, "started");
    // Nothing has arrived but the headers; dropping the stream ends the call.
    let _ = &mut stream;
    drop(stream);
    let how = next(&mut ended).await;
    assert!(how.contains("reset") || how.contains("closed"), "{how}");
}

#[rusty_tokio::test]
async fn a_sender_dropped_without_finishing_cancels_the_call() {
    let (server, mut ended) = stalling_server();
    let channel = channel(&server);
    let (mut sender, reply) = channel
        .client_streaming::<Blob, Blob>(PATH, rusty_tonic::client::Request::new(()))
        .await
        .unwrap();
    sender.send(&blob(1, 1)).await.unwrap();
    assert_eq!(next(&mut ended).await, "started");
    drop(sender);
    let err = reply.response().await.unwrap_err();
    assert_eq!(err.code(), Code::Cancelled);
    let how = next(&mut ended).await;
    assert!(how.contains("reset"), "the server saw {how}");
}

/// Whether the server noticed the call was over: a reset, or a stream already gone.
fn saw_the_call_end(how: &str) -> bool {
    how.contains("reset") || how.contains("closed")
}

#[rusty_tokio::test]
async fn a_reply_dropped_after_the_request_was_finished_cancels_the_call() {
    let (server, mut ended) = stalling_server();
    let (mut sender, reply) = channel(&server)
        .client_streaming::<Blob, Blob>(PATH, rusty_tonic::client::Request::new(()))
        .await
        .unwrap();
    sender.send(&blob(1, 1)).await.unwrap();
    sender.finish().await.unwrap();
    assert_eq!(next(&mut ended).await, "started");
    // The request is complete; only the reply is left to give up on.
    drop(reply);
    let how = next(&mut ended).await;
    assert!(saw_the_call_end(&how), "the server saw {how}");
}

#[rusty_tokio::test]
async fn a_reply_dropped_before_the_request_is_finished_still_ends_the_call() {
    let (server, mut ended) = stalling_server();
    let (mut sender, reply) = channel(&server)
        .client_streaming::<Blob, Blob>(PATH, rusty_tonic::client::Request::new(()))
        .await
        .unwrap();
    sender.send(&blob(1, 1)).await.unwrap();
    assert_eq!(next(&mut ended).await, "started");
    drop(reply);
    // The end of the request goes out at once, and only afterwards is the cancellation seen:
    // the call must still be reset, not left open with nobody holding it.
    let _ = sender.finish().await;
    let how = next(&mut ended).await;
    assert!(saw_the_call_end(&how), "the server saw {how}");
}

/// A server with room for one stream at a time. A call to `/pkg.Svc/Silent` is read whole and
/// then left unanswered; 300 ms on it tries to answer, which fails if the client has given up
/// (the stream was reset), and that is reported. Any other call is echoed.
fn one_stream_server() -> (rusty_hyper::Server, mpsc::Receiver<String>) {
    let mut h2 = Builder::new();
    h2.max_concurrent_streams(1);
    let options = ServeOptions {
        h2,
        ..ServeOptions::default()
    };
    let (tx, rx) = mpsc::channel(8);
    let server = serve(
        listener(),
        None,
        options,
        move |mut req: Request, respond: Respond| {
            let tx = tx.clone();
            async move {
                if header(&req.headers, ":path") != Some("/pkg.Svc/Silent") {
                    return echo(req, respond).await;
                }
                let _ = read_call(&mut req).await;
                tx.send("started".to_string()).await.unwrap();
                sleep(Duration::from_millis(300)).await;
                let answered = respond
                    .send_response(fields(&[(":status".into(), b"200".to_vec())]), true)
                    .await;
                tx.send(
                    if answered.is_ok() {
                        "answered"
                    } else {
                        "reset"
                    }
                    .to_string(),
                )
                .await
                .unwrap();
            }
        },
    )
    .unwrap();
    (server, rx)
}

#[rusty_tokio::test]
async fn a_response_wait_given_up_on_from_outside_after_the_request_was_finished_cancels_the_call()
{
    let (server, mut ended) = one_stream_server();
    let channel = channel(&server);
    let (mut sender, reply) = channel
        .client_streaming::<Blob, Blob>("/pkg.Svc/Silent", rusty_tonic::client::Request::new(()))
        .await
        .unwrap();
    sender.send(&blob(1, 1)).await.unwrap();
    sender.finish().await.unwrap();
    assert_eq!(next(&mut ended).await, "started");
    // The caller gives up while the server has sent no headers: no deadline of ours is involved.
    assert!(timeout(Duration::from_millis(100), reply.response())
        .await
        .is_err());
    let second = timeout(FIVE, channel.unary::<Blob, Blob>(PATH, request(blob(2, 2))))
        .await
        .expect("the stream slot was never freed")
        .unwrap();
    assert_eq!(second.message.n, 2);
    assert_eq!(
        next(&mut ended).await,
        "reset",
        "the server found the call reset"
    );
}

#[rusty_tokio::test]
async fn a_unary_call_given_up_on_from_outside_while_waiting_for_headers_cancels_the_call() {
    let (server, mut ended) = one_stream_server();
    let channel = channel(&server);
    let call = channel.unary::<Blob, Blob>("/pkg.Svc/Silent", request(blob(1, 1)));
    assert!(timeout(Duration::from_millis(100), call).await.is_err());
    assert_eq!(next(&mut ended).await, "started");
    let second = timeout(FIVE, channel.unary::<Blob, Blob>(PATH, request(blob(2, 2))))
        .await
        .expect("the stream slot was never freed")
        .unwrap();
    assert_eq!(second.message.n, 2);
    assert_eq!(
        next(&mut ended).await,
        "reset",
        "the server found the call reset"
    );
}

#[rusty_tokio::test]
async fn a_deadline_after_the_request_was_finished_cancels_the_call() {
    let (server, mut ended) = stalling_server();
    let mut request = rusty_tonic::client::Request::new(());
    request.timeout = Some(Duration::from_millis(600));
    let (mut sender, reply) = channel(&server)
        .client_streaming::<Blob, Blob>(PATH, request)
        .await
        .unwrap();
    sender.send(&blob(1, 1)).await.unwrap();
    sender.finish().await.unwrap();
    let err = reply.response().await.unwrap_err();
    assert_eq!(err.code(), Code::DeadlineExceeded);
    assert_eq!(next(&mut ended).await, "started");
    let how = next(&mut ended).await;
    assert!(saw_the_call_end(&how), "the server saw {how}");
}

#[rusty_tokio::test]
async fn a_finished_call_that_is_given_up_on_frees_its_stream_slot() {
    // One stream at a time: a second call can start only if the first one really ended.
    let mut h2 = Builder::new();
    h2.max_concurrent_streams(1);
    let options = ServeOptions {
        h2,
        ..ServeOptions::default()
    };
    let (tx, mut ended) = mpsc::channel(8);
    let server = serve(
        listener(),
        None,
        options,
        move |req: Request, respond: Respond| {
            let tx = tx.clone();
            async move {
                if header(&req.headers, ":path") == Some("/pkg.Svc/Stall") {
                    stall(req, respond, tx).await;
                } else {
                    echo(req, respond).await;
                }
            }
        },
    )
    .unwrap();
    let channel = channel(&server);
    let (mut sender, reply) = channel
        .client_streaming::<Blob, Blob>("/pkg.Svc/Stall", rusty_tonic::client::Request::new(()))
        .await
        .unwrap();
    sender.send(&blob(1, 1)).await.unwrap();
    sender.finish().await.unwrap();
    assert_eq!(next(&mut ended).await, "started");
    drop(reply);
    let second = timeout(FIVE, channel.unary::<Blob, Blob>(PATH, request(blob(2, 2))))
        .await
        .expect("the second call was never admitted: the first still holds the only stream")
        .unwrap();
    assert_eq!(second.message.n, 2);
}

#[rusty_tokio::test]
async fn concurrent_calls_share_one_connection() {
    // The server takes one connection; a second would be closed unserved.
    let options = ServeOptions {
        max_connections: 1,
        ..ServeOptions::default()
    };
    let server = serve(listener(), None, options, echo_slowly).unwrap();
    let channel = channel(&server);
    let calls: Vec<_> = (0..8u32)
        .map(|n| {
            let channel = channel.clone();
            rusty_tokio::spawn(async move {
                channel
                    .unary::<Blob, Blob>(PATH, request(blob(n, 4)))
                    .await
                    .map(|r| r.message.n)
            })
        })
        .collect();
    let mut got = Vec::new();
    for call in calls {
        got.push(call.await.unwrap().unwrap());
    }
    got.sort_unstable();
    assert_eq!(got, (0..8).collect::<Vec<_>>());
}

async fn echo_slowly(req: Request, respond: Respond) {
    sleep(Duration::from_millis(50)).await;
    echo(req, respond).await;
}

#[rusty_tokio::test]
async fn calls_past_the_servers_stream_limit_wait_their_turn() {
    let mut h2 = Builder::new();
    h2.max_concurrent_streams(2);
    let options = ServeOptions {
        h2,
        ..ServeOptions::default()
    };
    let server = serve(listener(), None, options, echo_slowly).unwrap();
    let channel = channel(&server);
    let calls: Vec<_> = (0..6u32)
        .map(|n| {
            let channel = channel.clone();
            rusty_tokio::spawn(async move {
                channel
                    .unary::<Blob, Blob>(PATH, request(blob(n, 4)))
                    .await
                    .map(|r| r.message.n)
            })
        })
        .collect();
    for call in calls {
        call.await.unwrap().unwrap();
    }
}

// ---- reconnecting ---------------------------------------------------------------------------

#[rusty_tokio::test]
async fn a_channel_reconnects_after_its_server_goes_away_and_comes_back() {
    let server = start(echo);
    let addr = server.local_addr();
    let channel = Channel::new(&url(addr), ChannelOptions::default()).unwrap();
    channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap();

    server.shutdown().await;
    let err = channel
        .unary::<Blob, Blob>(PATH, request(blob(2, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unavailable);

    let l = rusty_tokio::io::TcpListener::bind(addr).await.unwrap();
    let again = serve(l, None, ServeOptions::default(), echo).unwrap();
    let reply = channel
        .unary::<Blob, Blob>(PATH, request(blob(3, 1)))
        .await
        .unwrap();
    assert_eq!(reply.message.n, 3);
    again.shutdown().await;
}

#[rusty_tokio::test]
async fn a_call_made_after_the_server_said_goodbye_is_retried_on_a_new_connection() {
    // Two servers, one after the other, on one address: the first connection is told to go
    // away between calls, so the second call finds it closing and must open another.
    let server = start(echo);
    let addr = server.local_addr();
    let channel = Channel::new(&url(addr), ChannelOptions::default()).unwrap();
    channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap();
    server.shutdown().await;
    let l = rusty_tokio::io::TcpListener::bind(addr).await.unwrap();
    let again = serve(l, None, ServeOptions::default(), echo).unwrap();
    // Let the client take in the goodbye: its connection is closed, and nothing is sent on it.
    sleep(SHORT).await;
    let reply = channel
        .unary::<Blob, Blob>(PATH, request(blob(2, 1)))
        .await
        .unwrap();
    assert_eq!(reply.message.n, 2);
    again.shutdown().await;
}

#[rusty_tokio::test]
async fn a_call_in_flight_when_the_connection_dies_is_unavailable() {
    let l = listener();
    let addr = l.local_addr().unwrap();
    let (opened_tx, opened_rx) = oneshot::channel();
    rusty_tokio::spawn(async move {
        let (tcp, _) = l.accept().await.unwrap();
        let (mut acceptor, driver) = rusty_h2::driver::server(tcp, Builder::new());
        let driver = rusty_tokio::spawn(driver);
        let accepted = acceptor.accept().await.unwrap().unwrap();
        // Drop the connection under the call; once the driver is gone nothing more is written.
        driver.abort();
        let _ = driver.await;
        let _ = opened_tx.send(());
        drop(accepted);
        drop(acceptor);
    });
    let channel = Channel::new(&url(addr), ChannelOptions::default()).unwrap();
    let err = channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unavailable, "{err:?}");
    opened_rx.await.unwrap();
}

#[rusty_tokio::test]
async fn a_call_is_never_sent_twice() {
    // The server counts requests that reach it; a call that failed after the server saw it
    // must not be repeated by the client.
    let (tx, mut rx) = mpsc::channel(8);
    let l = listener();
    let addr = l.local_addr().unwrap();
    rusty_tokio::spawn(async move {
        let (tcp, _) = l.accept().await.unwrap();
        let (mut acceptor, driver) = rusty_h2::driver::server(tcp, Builder::new());
        let driver = rusty_tokio::spawn(driver);
        if let Some(Ok((mut req, _respond))) = acceptor.accept().await {
            let _ = read_call(&mut req).await;
            tx.send(()).await.unwrap();
            driver.abort();
            let _ = driver.await;
        }
    });
    let channel = Channel::new(&url(addr), ChannelOptions::default()).unwrap();
    let err = channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unavailable, "{err:?}");
    assert!(rx.recv().await.is_some(), "the server saw the call once");
    // The server task ends after the one call; a second would have been queued before that.
    sleep(SHORT).await;
    assert!(rx.recv().await.is_none(), "and not a second time");
}
