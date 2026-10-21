//! The client's calls against a scripted gRPC server (built from the core's own pieces over
//! `rusty_hyper::serve`): what is sent, what comes back, and how each failure becomes a status.
#![allow(clippy::unwrap_used)]

mod client_common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use client_common::*;
use rusty_hyper::{HeaderField, Request, Respond};
use rusty_prost::Message;
use rusty_tonic::client::{Channel, ChannelOptions};
use rusty_tonic::{Code, FrameDecoder, MetadataMap, Status};

const PATH: &str = "/pkg.Svc/Method";

fn request(blob: Blob) -> rusty_tonic::client::Request<Blob> {
    rusty_tonic::client::Request::new(blob)
}

// ---- unary ----------------------------------------------------------------------------------

#[rusty_tokio::test]
async fn a_unary_call_round_trips_with_metadata_and_trailers() {
    let server = start(|mut req: Request, respond: Respond| async move {
        let call = read_call(&mut req).await;
        // What the client sent.
        assert_eq!(call.raw[0].name, b":method");
        assert_eq!(header(&call.raw, ":method"), Some("POST"));
        assert_eq!(header(&call.raw, ":path"), Some("/pkg.Svc/Method"));
        assert_eq!(header(&call.raw, ":scheme"), Some("http"));
        assert_eq!(header(&call.raw, "content-type"), Some("application/grpc"));
        assert_eq!(header(&call.raw, "te"), Some("trailers"));
        assert_eq!(header(&call.raw, "grpc-accept-encoding"), Some("identity"));
        assert!(header(&call.raw, "user-agent")
            .unwrap()
            .starts_with("rusty-tonic/"));
        assert_eq!(header(&call.raw, "grpc-timeout"), None);
        assert_eq!(call.head.metadata.get("x-token"), Some("abc"));
        assert_eq!(call.blobs(), vec![blob(7, 100)]);
        // Initial metadata, one message, trailing metadata.
        let mut initial = MetadataMap::new();
        initial.insert("x-reply", "1").unwrap();
        let mut send = start_response(respond, &initial).await;
        send_message(&mut send, &blob(8, 50)).await;
        let mut end = MetadataMap::new();
        end.insert("x-end", "2").unwrap();
        finish_response(send, &Status::ok().with_metadata(end)).await;
    });
    let mut req = request(blob(7, 100));
    req.metadata.insert("x-token", "abc").unwrap();
    let reply = channel(&server)
        .unary::<Blob, Blob>(PATH, req)
        .await
        .unwrap();
    assert_eq!(reply.message, blob(8, 50));
    assert_eq!(reply.metadata.get("x-reply"), Some("1"));
    assert_eq!(reply.trailers.get("x-end"), Some("2"));
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_failed_call_carries_its_code_message_details_and_metadata() {
    let server = start(|mut req: Request, respond: Respond| async move {
        read_call(&mut req).await;
        let mut meta = MetadataMap::new();
        meta.insert("x-why", "because").unwrap();
        let status = Status::with_error_info(
            Code::NotFound,
            "no such thing: caf\u{e9} 100%",
            "MISSING",
            "example.test",
            BTreeMap::from([("k".to_string(), "v".to_string())]),
        )
        .with_metadata(meta);
        reply(respond, &[], &status).await;
    });
    let err = channel(&server)
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    assert_eq!(err.message(), "no such thing: caf\u{e9} 100%");
    assert_eq!(err.metadata().get("x-why"), Some("because"));
    let info = err.error_info().expect("details");
    assert_eq!(info.reason, "MISSING");
    assert_eq!(info.domain, "example.test");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_trailers_only_response_is_the_calls_status() {
    let server = start(|mut req: Request, respond: Respond| async move {
        read_call(&mut req).await;
        trailers_only(respond, &Status::permission_denied("not for you")).await;
    });
    let err = channel(&server)
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert_eq!(err.message(), "not for you");
    server.shutdown().await;
}

/// A server that answers by path with a response a real server should not send.
async fn misbehave(mut req: Request, respond: Respond) {
    let path = header(&req.headers, ":path").unwrap().to_string();
    read_call(&mut req).await;
    let ok = || HeaderField::new(":status", "200");
    let grpc = || HeaderField::new("content-type", "application/grpc");
    match path.as_str() {
        "/bad/503" => {
            respond
                .send_response(vec![HeaderField::new(":status", "503")], true)
                .await
                .unwrap();
        }
        "/bad/text" => {
            let fields = vec![ok(), HeaderField::new("content-type", "text/plain")];
            respond.send_response(fields, true).await.unwrap();
        }
        "/bad/gzip" => {
            let fields = vec![ok(), grpc(), HeaderField::new("grpc-encoding", "gzip")];
            let mut send = respond.send_response(fields, false).await.unwrap();
            send.send_data(vec![1, 0, 0, 0, 1, 0], false).await.unwrap();
            finish_response(send, &Status::ok()).await;
        }
        "/bad/no-status" => {
            let mut send = respond
                .send_response(vec![ok(), grpc()], false)
                .await
                .unwrap();
            let frame = rusty_tonic::encode_message(&blob(1, 3)).unwrap();
            send.send_data(frame, true).await.unwrap();
        }
        "/bad/cut" => {
            let mut send = respond
                .send_response(vec![ok(), grpc()], false)
                .await
                .unwrap();
            // A prefix promising 10 bytes, then 3.
            send.send_data(vec![0, 0, 0, 0, 10, 1, 2, 3], false)
                .await
                .unwrap();
            finish_response(send, &Status::ok()).await;
        }
        "/bad/two" => reply(respond, &[blob(1, 1), blob(2, 1)], &Status::ok()).await,
        "/bad/none" => reply(respond, &[], &Status::ok()).await,
        other => panic!("unexpected path {other}"),
    }
}

#[rusty_tokio::test]
async fn replies_a_server_should_not_send_become_statuses() {
    let server = start(misbehave);
    let channel = channel(&server);
    for (path, code) in [
        ("/bad/503", Code::Unavailable),
        ("/bad/text", Code::Unknown),
        ("/bad/gzip", Code::Unimplemented),
        ("/bad/no-status", Code::Unknown),
        ("/bad/cut", Code::Internal),
        ("/bad/two", Code::Internal),
        ("/bad/none", Code::Internal),
    ] {
        let err = channel
            .unary::<Blob, Blob>(path, request(blob(1, 1)))
            .await
            .unwrap_err();
        assert_eq!(err.code(), code, "{path}: {err}");
    }
    server.shutdown().await;
}

// ---- streaming ------------------------------------------------------------------------------

#[rusty_tokio::test]
async fn a_server_stream_yields_its_messages_then_ok_and_trailers() {
    let server = start(|mut req: Request, respond: Respond| async move {
        read_call(&mut req).await;
        let mut initial = MetadataMap::new();
        initial.insert("x-start", "s").unwrap();
        let mut send = start_response(respond, &initial).await;
        for n in 1..=3 {
            send_message(&mut send, &blob(n, 10)).await;
        }
        let mut end = MetadataMap::new();
        end.insert("x-done", "d").unwrap();
        finish_response(send, &Status::ok().with_metadata(end)).await;
    });
    let mut stream = channel(&server)
        .server_streaming::<Blob, Blob>(PATH, request(blob(0, 0)))
        .await
        .unwrap();
    assert_eq!(stream.metadata().get("x-start"), Some("s"));
    assert!(stream.trailers().is_none(), "the call has not ended");
    for n in 1..=3 {
        assert_eq!(stream.message().await.unwrap(), Some(blob(n, 10)));
    }
    assert_eq!(stream.message().await.unwrap(), None);
    assert_eq!(stream.trailers().unwrap().get("x-done"), Some("d"));
    // The end stays the end.
    assert_eq!(stream.message().await.unwrap(), None);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_server_stream_that_fails_delivers_what_came_first_then_the_status() {
    let server = start(|mut req: Request, respond: Respond| async move {
        read_call(&mut req).await;
        reply(
            respond,
            &[blob(1, 5), blob(2, 5)],
            &Status::aborted("cut off"),
        )
        .await;
    });
    let mut stream = channel(&server)
        .server_streaming::<Blob, Blob>(PATH, request(blob(0, 0)))
        .await
        .unwrap();
    assert_eq!(stream.message().await.unwrap(), Some(blob(1, 5)));
    assert_eq!(stream.message().await.unwrap(), Some(blob(2, 5)));
    for _ in 0..2 {
        let err = stream.message().await.unwrap_err();
        assert_eq!((err.code(), err.message()), (Code::Aborted, "cut off"));
    }
    assert!(stream.trailers().is_some());
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_server_stream_that_fails_at_once_fails_the_call_that_opens_it() {
    let server = start(|mut req: Request, respond: Respond| async move {
        read_call(&mut req).await;
        trailers_only(respond, &Status::unauthenticated("who?")).await;
    });
    let err = channel(&server)
        .server_streaming::<Blob, Blob>(PATH, request(blob(0, 0)))
        .await
        .err()
        .expect("a trailers-only response is an error");
    assert_eq!(err.code(), Code::Unauthenticated);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_client_stream_sends_every_message_and_gets_one_reply() {
    let server = start(|mut req: Request, respond: Respond| async move {
        let call = read_call(&mut req).await;
        let total: u32 = call.blobs().iter().map(|b| b.n).sum();
        reply(
            respond,
            &[Blob {
                data: vec![],
                n: total,
            }],
            &Status::ok(),
        )
        .await;
    });
    let (mut tx, reply) = channel(&server)
        .client_streaming::<Blob, Blob>(PATH, rusty_tonic::client::Request::new(()))
        .await
        .unwrap();
    for n in 1..=4 {
        tx.send(&blob(n, 1_000)).await.unwrap();
    }
    tx.finish().await.unwrap();
    let response = reply.response().await.unwrap();
    assert_eq!(response.message.n, 10);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_bidirectional_call_interleaves_requests_and_replies() {
    // The server answers each message as it arrives, before the next is sent.
    let server = start(|mut req: Request, respond: Respond| async move {
        let mut send = start_response(respond, &MetadataMap::new()).await;
        let mut decoder = FrameDecoder::new(usize::MAX);
        while let Some(m) = next_message(&mut req.body, &mut decoder).await {
            let mut b = Blob::decode(&m).unwrap();
            b.n += 100;
            send_message(&mut send, &b).await;
        }
        finish_response(send, &Status::ok()).await;
    });
    let (mut tx, mut rx) = channel(&server)
        .bidirectional::<Blob, Blob>(PATH, rusty_tonic::client::Request::new(()))
        .await
        .unwrap();
    for n in 1..=5 {
        tx.send(&blob(n, 20)).await.unwrap();
        // The reply to this message arrives before the next is sent.
        let got = rx.message().await.unwrap().unwrap();
        assert_eq!(got.n, n + 100);
    }
    tx.finish().await.unwrap();
    assert_eq!(rx.message().await.unwrap(), None);
    server.shutdown().await;
}

// ---- limits, the interceptor, bad arguments ----------------------------------------------------

/// A blob whose encoded message is exactly `size` bytes.
fn blob_encoding_to(size: usize) -> Blob {
    let b = Blob {
        data: vec![7; size - 5],
        n: 0,
    };
    assert_eq!(b.encode_to_vec().unwrap().len(), size);
    b
}

#[rusty_tokio::test]
async fn a_reply_at_the_receive_limit_passes_and_one_byte_over_does_not() {
    let limit = 4 * 1024 * 1024;
    let server = start(|mut req: Request, respond: Respond| async move {
        let call = read_call(&mut req).await;
        let size = call.blobs()[0].n as usize;
        reply(respond, &[blob_encoding_to(size)], &Status::ok()).await;
    });
    let channel = channel(&server);
    let ok = channel
        .unary::<Blob, Blob>(
            PATH,
            request(Blob {
                data: vec![],
                n: limit as u32,
            }),
        )
        .await
        .unwrap();
    assert_eq!(ok.message.encode_to_vec().unwrap().len(), limit);
    let err = channel
        .unary::<Blob, Blob>(
            PATH,
            request(Blob {
                data: vec![],
                n: limit as u32 + 1,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::ResourceExhausted);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_request_over_the_send_limit_fails_before_anything_is_sent() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let server = start(move |mut req: Request, respond: Respond| {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            read_call(&mut req).await;
            reply(respond, &[], &Status::ok()).await;
        }
    });
    let options = ChannelOptions {
        max_send_message_size: Some(100),
        ..ChannelOptions::default()
    };
    let channel = Channel::new(&format!("http://{}", server.local_addr()), options).unwrap();
    let err = channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 500)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::ResourceExhausted);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn the_interceptor_adds_metadata_and_can_refuse_a_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let server = start(move |mut req: Request, respond: Respond| {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            let call = read_call(&mut req).await;
            let token = call.head.metadata.get("authorization").unwrap().to_string();
            let b = Blob {
                data: token.into_bytes(),
                n: 0,
            };
            reply(respond, &[b], &Status::ok()).await;
        }
    });
    let refuse = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = refuse.clone();
    let options = ChannelOptions {
        interceptor: Some(Arc::new(move |m: &mut MetadataMap| {
            if flag.load(Ordering::SeqCst) {
                return Err(Status::unauthenticated("no token"));
            }
            m.insert("authorization", "Bearer t0k").unwrap();
            Ok(())
        })),
        ..ChannelOptions::default()
    };
    let channel = Channel::new(&format!("http://{}", server.local_addr()), options).unwrap();
    let ok = channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap();
    assert_eq!(ok.message.data, b"Bearer t0k");
    refuse.store(true, Ordering::SeqCst);
    let err = channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(
        (err.code(), err.message()),
        (Code::Unauthenticated, "no token")
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a refused call sends nothing"
    );
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn an_unusable_url_is_invalid_argument_and_a_refused_connection_is_unavailable() {
    for url in ["not a url", "ftp://127.0.0.1:1/", ""] {
        let err = Channel::new(url, ChannelOptions::default()).err().unwrap();
        assert_eq!(err.code(), Code::InvalidArgument, "{url}");
    }
    let l = listener();
    let addr = l.local_addr().unwrap();
    drop(l);
    // Creating the channel does no I/O; the call finds nobody there.
    let channel = Channel::new(&format!("http://{addr}"), ChannelOptions::default()).unwrap();
    let err = channel
        .unary::<Blob, Blob>(PATH, request(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unavailable);
}
