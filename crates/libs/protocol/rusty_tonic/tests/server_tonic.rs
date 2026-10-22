//! The server against a real `tonic` client (the oracle): every call shape, an error with
//! metadata, a deadline, and the receive limit.
#![allow(clippy::unwrap_used)]

mod client_common;
mod server_common;

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use http::uri::PathAndQuery;
use rusty_tonic::client::{Request, Response};
use rusty_tonic::server::{Incoming, Router, Sink};
use rusty_tonic::{Code, Status};
use server_common::serve;
use tonic_prost::ProstCodec;

#[derive(Clone, PartialEq, prost::Message)]
struct PBlob {
    #[prost(bytes = "vec", tag = "1")]
    data: Vec<u8>,
    #[prost(uint32, tag = "2")]
    n: u32,
}

/// The same message as `client_common::Blob`, for our side of the wire.
use client_common::Blob;

fn router() -> Router {
    Router::new()
        .unary("/pkg.Svc/Unary", |r: Request<Blob>| async move {
            if r.message.n == 99 {
                let mut m = rusty_tonic::MetadataMap::new();
                m.insert("x-why", "missing").unwrap();
                return Err(Status::not_found("gone").with_metadata(m));
            }
            let mut reply = Response::new(Blob {
                n: r.message.n + 1,
                data: r.message.data,
            });
            reply.metadata.insert("x-reply", "yes").unwrap();
            Ok(reply)
        })
        .unary("/pkg.Svc/Slow", |r: Request<Blob>| async move {
            let _dropped = Dropped;
            rusty_tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(Response::new(r.message))
        })
        .server_streaming(
            "/pkg.Svc/ServerStream",
            |r: Request<Blob>, mut out: Sink<Blob>| async move {
                for n in 0..r.message.n {
                    out.send(&Blob {
                        data: vec![n as u8; 3],
                        n,
                    })
                    .await?;
                }
                Ok(())
            },
        )
        .client_streaming(
            "/pkg.Svc/ClientStream",
            |r: Request<Incoming<Blob>>| async move {
                let mut incoming = r.message;
                let mut sum = 0;
                while let Some(b) = incoming.message().await? {
                    sum += b.n;
                }
                Ok(Response::new(Blob {
                    data: Vec::new(),
                    n: sum,
                }))
            },
        )
        .bidirectional(
            "/pkg.Svc/Bidi",
            |r: Request<Incoming<Blob>>, mut out: Sink<Blob>| async move {
                let mut incoming = r.message;
                while let Some(b) = incoming.message().await? {
                    out.send(&Blob { n: b.n + 100, ..b }).await?;
                }
                Ok(())
            },
        )
}

fn runtimes() -> (rusty_tokio::Runtime, tokio::runtime::Runtime) {
    (
        rusty_tokio::Runtime::new().unwrap(),
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap(),
    )
}

/// Runs `f` on the oracle runtime with a `tonic` client connected to our server.
fn with_client<F, Fut>(f: F)
where
    F: FnOnce(tonic::client::Grpc<tonic::transport::Channel>) -> Fut,
    Fut: Future<Output = ()>,
{
    let (ours, oracle) = runtimes();
    let server = ours.block_on(async { serve(router()) });
    let addr = server.local_addr();
    oracle.block_on(async move {
        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut grpc = tonic::client::Grpc::new(channel);
        grpc.ready().await.unwrap();
        tokio::time::timeout(Duration::from_secs(20), f(grpc))
            .await
            .unwrap();
    });
    ours.block_on(server.shutdown());
}

fn path(p: &'static str) -> PathAndQuery {
    PathAndQuery::from_static(p)
}

fn codec() -> ProstCodec<PBlob, PBlob> {
    ProstCodec::default()
}

fn pblob(n: u32, len: usize) -> PBlob {
    PBlob {
        data: vec![7; len],
        n,
    }
}

#[test]
fn a_unary_call_and_its_metadata() {
    with_client(|mut g| async move {
        let mut req = tonic::Request::new(pblob(7, 1000));
        req.metadata_mut().insert("x-in", "1".parse().unwrap());
        let r = g.unary(req, path("/pkg.Svc/Unary"), codec()).await.unwrap();
        assert_eq!(r.metadata().get("x-reply").unwrap(), "yes");
        assert_eq!(r.into_inner().n, 8);
    });
}

#[test]
fn an_error_with_metadata_and_an_unimplemented_method() {
    with_client(|mut g| async move {
        let e = g
            .unary(
                tonic::Request::new(pblob(99, 0)),
                path("/pkg.Svc/Unary"),
                codec(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code(), tonic::Code::NotFound);
        assert_eq!(e.message(), "gone");
        assert_eq!(e.metadata().get("x-why").unwrap(), "missing");

        g.ready().await.unwrap();
        let e = g
            .unary(
                tonic::Request::new(pblob(1, 0)),
                path("/pkg.Svc/Nope"),
                codec(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code(), tonic::Code::Unimplemented);
    });
}

#[test]
fn a_server_stream() {
    with_client(|mut g| async move {
        let mut s = g
            .server_streaming(
                tonic::Request::new(pblob(5, 0)),
                path("/pkg.Svc/ServerStream"),
                codec(),
            )
            .await
            .unwrap()
            .into_inner();
        for i in 0..5 {
            assert_eq!(s.message().await.unwrap().unwrap().n, i);
        }
        assert!(s.message().await.unwrap().is_none());
    });
}

#[test]
fn a_client_stream() {
    with_client(|mut g| async move {
        let items = futures_util::stream::iter((1..=10).map(|n| pblob(n, 100)));
        let r = g
            .client_streaming(
                tonic::Request::new(items),
                path("/pkg.Svc/ClientStream"),
                codec(),
            )
            .await
            .unwrap();
        assert_eq!(r.into_inner().n, 55);
    });
}

#[test]
fn a_bidirectional_stream() {
    with_client(|mut g| async move {
        let items = futures_util::stream::iter((0..20).map(|n| pblob(n, 10)));
        let mut out = g
            .streaming(tonic::Request::new(items), path("/pkg.Svc/Bidi"), codec())
            .await
            .unwrap()
            .into_inner();
        let mut got = Vec::new();
        while let Some(b) = out.next().await {
            got.push(b.unwrap().n);
        }
        assert_eq!(got, (100..120).collect::<Vec<_>>());
    });
}

/// Set when the slow handler's future is dropped.
static SLOW_DROPPED: AtomicBool = AtomicBool::new(false);

struct Dropped;

impl Drop for Dropped {
    fn drop(&mut self) {
        SLOW_DROPPED.store(true, Ordering::SeqCst);
    }
}

#[test]
fn the_clients_deadline_stops_a_slow_handler() {
    with_client(|mut g| async move {
        let mut req = tonic::Request::new(pblob(1, 1));
        req.set_timeout(Duration::from_millis(300));
        // `tonic` also times the call out itself; what matters is that our server dropped the
        // handler, which a cancel alone would not (a sleeping handler is only dropped by its
        // deadline).
        assert!(g.unary(req, path("/pkg.Svc/Slow"), codec()).await.is_err());
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(SLOW_DROPPED.load(Ordering::SeqCst));
    });
}

#[test]
fn a_request_over_the_receive_limit_is_resource_exhausted() {
    with_client(|mut g| async move {
        let e = g
            .unary(
                tonic::Request::new(pblob(1, 5 * 1024 * 1024)),
                path("/pkg.Svc/Unary"),
                codec(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code(), tonic::Code::ResourceExhausted, "{e}");
        let _ = Code::Ok;
    });
}
