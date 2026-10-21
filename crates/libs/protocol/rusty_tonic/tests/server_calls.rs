//! The server's calls: every shape through this crate's own client, the errors a handler can
//! return, and what a misbehaving client gets (a bare HTTP/2 client sends the exact bytes).
#![allow(clippy::unwrap_used)]

mod client_common;
mod server_common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use client_common::*;
use rusty_prost::Message;
use rusty_tonic::client::{Channel, Request, Response};
use rusty_tonic::server::{Incoming, Router, Sink};
use rusty_tonic::{Code, MetadataMap, Status};
use server_common::*;

const PATH: &str = "/pkg.Svc/Method";

fn req(blob: Blob) -> Request<Blob> {
    Request::new(blob)
}

fn client(server: &rusty_hyper::Server) -> Channel {
    channel(server)
}

// ---- the four shapes ------------------------------------------------------------------------

#[rusty_tokio::test]
async fn a_unary_call_round_trips_with_metadata_and_trailers() {
    let router = Router::new().unary(PATH, |r: Request<Blob>| async move {
        assert_eq!(r.metadata.get("x-in"), Some("1"));
        let mut metadata = MetadataMap::new();
        metadata.insert("x-start", "s").unwrap();
        let mut trailers = MetadataMap::new();
        trailers.insert("x-done", "d").unwrap();
        Ok(Response {
            message: Blob {
                n: r.message.n + 1,
                data: r.message.data,
            },
            metadata,
            trailers,
        })
    });
    let server = serve(router);
    let mut r = req(blob(7, 1000));
    r.metadata.insert("x-in", "1").unwrap();
    let reply = client(&server).unary::<Blob, Blob>(PATH, r).await.unwrap();
    assert_eq!(
        reply.message,
        Blob {
            n: 8,
            ..blob(7, 1000)
        }
    );
    assert_eq!(reply.metadata.get("x-start"), Some("s"));
    assert_eq!(reply.trailers.get("x-done"), Some("d"));
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_server_stream_sends_its_replies_then_the_status() {
    let router =
        Router::new().server_streaming(PATH, |r: Request<Blob>, mut out: Sink<Blob>| async move {
            let mut initial = MetadataMap::new();
            initial.insert("x-start", "s").unwrap();
            out.set_metadata(initial);
            let mut end = MetadataMap::new();
            end.insert("x-done", "d").unwrap();
            out.set_trailers(end);
            for n in 0..r.message.n {
                out.send(&blob(n, 100)).await?;
            }
            Ok(())
        });
    let server = serve(router);
    let mut stream = client(&server)
        .server_streaming::<Blob, Blob>(PATH, req(blob(5, 0)))
        .await
        .unwrap();
    for n in 0..5 {
        assert_eq!(stream.message().await.unwrap(), Some(blob(n, 100)));
    }
    assert_eq!(stream.message().await.unwrap(), None);
    assert_eq!(stream.metadata().get("x-start"), Some("s"));
    assert_eq!(stream.trailers().unwrap().get("x-done"), Some("d"));
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_client_stream_is_read_to_its_end_for_one_reply() {
    let router = Router::new().client_streaming(PATH, |r: Request<Incoming<Blob>>| async move {
        let mut incoming = r.message;
        let mut sum = 0;
        while let Some(b) = incoming.message().await? {
            sum += b.n;
        }
        Ok(Response::new(blob(sum, 0)))
    });
    let server = serve(router);
    let (mut tx, reply) = client(&server)
        .client_streaming::<Blob, Blob>(PATH, Request::new(()))
        .await
        .unwrap();
    for n in 1..=10 {
        tx.send(&blob(n, 50)).await.unwrap();
    }
    tx.finish().await.unwrap();
    assert_eq!(reply.response().await.unwrap().message.n, 55);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_bidirectional_call_interleaves_requests_and_replies() {
    let router = Router::new().bidirectional(
        PATH,
        |r: Request<Incoming<Blob>>, mut out: Sink<Blob>| async move {
            let mut incoming = r.message;
            while let Some(b) = incoming.message().await? {
                out.send(&Blob { n: b.n + 100, ..b }).await?;
            }
            Ok(())
        },
    );
    let server = serve(router);
    let (mut tx, mut rx) = client(&server)
        .bidirectional::<Blob, Blob>(PATH, Request::new(()))
        .await
        .unwrap();
    for n in 0..20 {
        tx.send(&blob(n, 10)).await.unwrap();
        assert_eq!(rx.message().await.unwrap().unwrap().n, n + 100);
    }
    tx.finish().await.unwrap();
    assert_eq!(rx.message().await.unwrap(), None);
    server.shutdown().await;
}

// ---- errors ---------------------------------------------------------------------------------

#[rusty_tokio::test]
async fn a_handler_error_is_the_calls_status_with_its_details_and_metadata() {
    let router = Router::new().unary(PATH, |_r: Request<Blob>| async move {
        let mut metadata = MetadataMap::new();
        metadata.insert("x-why", "missing").unwrap();
        Err::<Response<Blob>, _>(
            Status::with_error_info(
                Code::NotFound,
                "no such thing: ünïcode",
                "MISSING",
                "pkg",
                Default::default(),
            )
            .with_metadata(metadata),
        )
    });
    let server = serve(router);
    let err = client(&server)
        .unary::<Blob, Blob>(PATH, req(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    assert_eq!(err.message(), "no such thing: ünïcode");
    assert_eq!(err.metadata().get("x-why"), Some("missing"));
    assert_eq!(err.error_info().unwrap().reason, "MISSING");
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn an_error_before_anything_is_sent_is_a_trailers_only_response() {
    let router = Router::new().unary(PATH, |_r: Request<Blob>| async move {
        Err::<Response<Blob>, _>(Status::permission_denied("no"))
    });
    let server = serve(router);
    let r = raw(
        server.local_addr(),
        grpc_headers(PATH),
        &[frame(&blob(1, 1).encode_to_vec().unwrap())],
        true,
    )
    .await
    .unwrap();
    assert!(r.trailers_only());
    assert_eq!(r.status().code(), Code::PermissionDenied);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn an_error_after_replies_comes_as_trailers_after_them() {
    let router =
        Router::new().server_streaming(PATH, |_r: Request<Blob>, mut out: Sink<Blob>| async move {
            out.send(&blob(1, 1)).await?;
            out.send(&blob(2, 1)).await?;
            Err(Status::aborted("halfway"))
        });
    let server = serve(router);
    let mut stream = client(&server)
        .server_streaming::<Blob, Blob>(PATH, req(blob(0, 0)))
        .await
        .unwrap();
    assert_eq!(stream.message().await.unwrap().unwrap().n, 1);
    assert_eq!(stream.message().await.unwrap().unwrap().n, 2);
    let err = stream.message().await.unwrap_err();
    assert_eq!((err.code(), err.message()), (Code::Aborted, "halfway"));
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn an_unknown_method_is_unimplemented() {
    let server = serve(Router::new().unary(PATH, |r: Request<Blob>| async move {
        Ok(Response::new(r.message))
    }));
    let err = client(&server)
        .unary::<Blob, Blob>("/pkg.Svc/Other", req(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
    assert!(err.message().contains("/pkg.Svc/Other"), "{err}");
    server.shutdown().await;
}

// ---- what the handler sees ------------------------------------------------------------------

#[rusty_tokio::test]
async fn the_handler_sees_the_clients_deadline() {
    let router = Router::new().unary(PATH, |r: Request<Blob>| async move {
        let n = r.timeout.map_or(0, |t| t.as_secs() as u32);
        Ok(Response::new(blob(n, 0)))
    });
    let server = serve(router);
    let with = client(&server)
        .unary::<Blob, Blob>(PATH, req(blob(0, 0)).with_timeout(FIVE * 2))
        .await
        .unwrap();
    assert!((8..=10).contains(&with.message.n), "{}", with.message.n);
    let without = client(&server)
        .unary::<Blob, Blob>(PATH, req(blob(0, 0)))
        .await
        .unwrap();
    assert_eq!(without.message.n, 0);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn an_interceptor_can_refuse_a_call_before_the_handler_runs() {
    let ran = Arc::new(AtomicUsize::new(0));
    let counter = ran.clone();
    let router = Router::new()
        .unary(PATH, move |r: Request<Blob>| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(Response::new(r.message))
            }
        })
        .with_interceptor(Arc::new(|m: &mut MetadataMap| {
            if m.get("authorization") == Some("let me in") {
                Ok(())
            } else {
                Err(Status::unauthenticated("who are you"))
            }
        }));
    let server = serve(router);
    let c = client(&server);
    let err = c
        .unary::<Blob, Blob>(PATH, req(blob(1, 1)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
    assert_eq!(ran.load(Ordering::SeqCst), 0);

    let mut ok = req(blob(1, 1));
    ok.metadata.insert("authorization", "let me in").unwrap();
    c.unary::<Blob, Blob>(PATH, ok).await.unwrap();
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    server.shutdown().await;
}

// ---- limits ---------------------------------------------------------------------------------

#[rusty_tokio::test]
async fn a_request_over_the_receive_limit_is_resource_exhausted() {
    let router = Router::new()
        .unary(PATH, |r: Request<Blob>| async move {
            Ok(Response::new(r.message))
        })
        .with_max_recv_message_size(1000);
    let server = serve(router);
    let c = client(&server);
    c.unary::<Blob, Blob>(PATH, req(blob(1, 900)))
        .await
        .unwrap();
    let err = c
        .unary::<Blob, Blob>(PATH, req(blob(1, 2000)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::ResourceExhausted);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_reply_over_the_send_limit_fails_the_handlers_send() {
    let router = Router::new()
        .server_streaming(PATH, |_r: Request<Blob>, mut out: Sink<Blob>| async move {
            out.send(&blob(1, 10)).await?;
            let err = out.send(&blob(2, 5000)).await.unwrap_err();
            assert_eq!(err.code(), Code::ResourceExhausted);
            // The call goes on: the oversize message was refused, not the stream.
            out.send(&blob(3, 10)).await?;
            Ok(())
        })
        .with_max_send_message_size(1000);
    let server = serve(router);
    let mut stream = client(&server)
        .server_streaming::<Blob, Blob>(PATH, req(blob(0, 0)))
        .await
        .unwrap();
    assert_eq!(stream.message().await.unwrap().unwrap().n, 1);
    assert_eq!(stream.message().await.unwrap().unwrap().n, 3);
    assert_eq!(stream.message().await.unwrap(), None);
    server.shutdown().await;
}

// ---- clients that misbehave -----------------------------------------------------------------

fn echo_router() -> Router {
    Router::new().unary(PATH, |r: Request<Blob>| async move {
        Ok(Response::new(r.message))
    })
}

fn with(headers: Vec<(String, Vec<u8>)>, name: &str, value: &str) -> Vec<(String, Vec<u8>)> {
    let mut headers: Vec<_> = headers.into_iter().filter(|(n, _)| n != name).collect();
    headers.push((name.to_string(), value.as_bytes().to_vec()));
    headers
}

#[rusty_tokio::test]
async fn calls_that_are_not_grpc_get_a_status_not_a_hang() {
    let server = serve(echo_router());
    let addr = server.local_addr();
    let body = [frame(&blob(1, 1).encode_to_vec().unwrap())];
    for (headers, code) in [
        (
            with(grpc_headers(PATH), "content-type", "text/plain"),
            Code::Unimplemented,
        ),
        (with(grpc_headers(PATH), ":method", "GET"), Code::Internal),
        (
            with(grpc_headers(PATH), "grpc-encoding", "gzip"),
            Code::Unimplemented,
        ),
        (
            with(grpc_headers(PATH), "grpc-timeout", "soon"),
            Code::Internal,
        ),
        (
            with(grpc_headers(PATH), ":path", "/nonsense"),
            Code::Unimplemented,
        ),
    ] {
        let r = raw(addr, headers, &body, true).await.unwrap();
        assert_eq!(r.status().code(), code, "{:?}", r.status());
    }
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_unary_call_needs_exactly_one_message() {
    let server = serve(echo_router());
    let addr = server.local_addr();
    let one = frame(&blob(1, 1).encode_to_vec().unwrap());
    let none = raw(addr, grpc_headers(PATH), &[], true).await.unwrap();
    assert_eq!(none.status().code(), Code::Internal);
    let two = raw(addr, grpc_headers(PATH), &[one.clone(), one.clone()], true)
        .await
        .unwrap();
    assert_eq!(two.status().code(), Code::Internal);
    let ok = raw(addr, grpc_headers(PATH), &[one], true).await.unwrap();
    assert_eq!(ok.status().code(), Code::Ok);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_message_cut_short_or_compressed_is_refused() {
    let server = serve(echo_router());
    let addr = server.local_addr();
    // A prefix promising 10 bytes, then 3, then the end of the request.
    let cut = raw(
        addr,
        grpc_headers(PATH),
        &[vec![0, 0, 0, 0, 10, 1, 2, 3]],
        true,
    )
    .await
    .unwrap();
    assert_eq!(cut.status().code(), Code::Internal);
    // The per-message compressed flag set, with no encoding negotiated.
    let compressed = raw(addr, grpc_headers(PATH), &[vec![1, 0, 0, 0, 1, 0]], true)
        .await
        .unwrap();
    assert_eq!(compressed.status().code(), Code::Unimplemented);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn the_server_keeps_serving_after_misbehaving_clients() {
    let server = serve(echo_router());
    let addr = server.local_addr();
    for _ in 0..5 {
        let _ = raw(addr, grpc_headers(PATH), &[vec![9; 3]], true).await;
    }
    let reply = client(&server)
        .unary::<Blob, Blob>(PATH, req(blob(4, 4)))
        .await
        .unwrap();
    assert_eq!(reply.message.n, 4);
    server.shutdown().await;
}
