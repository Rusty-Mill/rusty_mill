//! The client against a real `tonic` server (the oracle): every call shape, an error with
//! details, and metadata both ways.
#![allow(clippy::unwrap_used)]

mod client_common;

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use client_common::*;
use rusty_stream::{Compat, StreamExt};
use rusty_tonic::client::{Channel, ChannelOptions, Request};
use rusty_tonic::Code;
use tonic::body::Body;
use tonic::server::{
    ClientStreamingService, Grpc, NamedService, ServerStreamingService, StreamingService,
    UnaryService,
};
use tonic_prost::ProstCodec;

#[derive(Clone, PartialEq, prost::Message)]
struct PBlob {
    #[prost(bytes = "vec", tag = "1")]
    data: Vec<u8>,
    #[prost(uint32, tag = "2")]
    n: u32,
}

#[derive(Clone)]
struct Svc;

impl NamedService for Svc {
    const NAME: &'static str = "pkg.Svc";
}

type Reply<T> = Pin<Box<dyn Future<Output = Result<tonic::Response<T>, tonic::Status>> + Send>>;
type Out = Pin<Box<dyn futures_core::Stream<Item = Result<PBlob, tonic::Status>> + Send>>;

struct UnaryM;
impl UnaryService<PBlob> for UnaryM {
    type Response = PBlob;
    type Future = Reply<PBlob>;
    fn call(&mut self, r: tonic::Request<PBlob>) -> Self::Future {
        Box::pin(async move {
            let mut b = r.into_inner();
            if b.n == 99 {
                let mut s = tonic::Status::not_found("gone");
                s.metadata_mut().insert("x-why", "missing".parse().unwrap());
                return Err(s);
            }
            b.n += 1;
            let mut resp = tonic::Response::new(b);
            resp.metadata_mut()
                .insert("x-reply", "yes".parse().unwrap());
            Ok(resp)
        })
    }
}

struct ServerStreamM;
impl ServerStreamingService<PBlob> for ServerStreamM {
    type Response = PBlob;
    type ResponseStream = Out;
    type Future = Reply<Out>;
    fn call(&mut self, r: tonic::Request<PBlob>) -> Self::Future {
        Box::pin(async move {
            let n = r.into_inner().n;
            let items = (0..n).map(|i| {
                Ok(PBlob {
                    data: vec![i as u8; 3],
                    n: i,
                })
            });
            let out: Out = Box::pin(Compat::new(rusty_stream::stream::iter(items)));
            Ok(tonic::Response::new(out))
        })
    }
}

struct ClientStreamM;
impl ClientStreamingService<PBlob> for ClientStreamM {
    type Response = PBlob;
    type Future = Reply<PBlob>;
    fn call(&mut self, r: tonic::Request<tonic::Streaming<PBlob>>) -> Self::Future {
        Box::pin(async move {
            let mut stream = r.into_inner();
            let mut sum = 0;
            while let Some(b) = stream.message().await? {
                sum += b.n;
            }
            Ok(tonic::Response::new(PBlob {
                data: Vec::new(),
                n: sum,
            }))
        })
    }
}

struct BidiM;
impl StreamingService<PBlob> for BidiM {
    type Response = PBlob;
    type ResponseStream = Out;
    type Future = Reply<Out>;
    fn call(&mut self, r: tonic::Request<tonic::Streaming<PBlob>>) -> Self::Future {
        Box::pin(async move {
            let out: Out = Box::pin(Compat::new(Compat::new(r.into_inner()).map(|b| {
                b.map(|mut b| {
                    b.n += 100;
                    b
                })
            })));
            Ok(tonic::Response::new(out))
        })
    }
}

type Fut = Pin<Box<dyn Future<Output = Result<http::Response<Body>, Infallible>> + Send>>;

impl tower::Service<http::Request<Body>> for Svc {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = Fut;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<Body>) -> Fut {
        let mut grpc = Grpc::new(ProstCodec::<PBlob, PBlob>::default());
        match req.uri().path() {
            "/pkg.Svc/Unary" => Box::pin(async move { Ok(grpc.unary(UnaryM, req).await) }),
            "/pkg.Svc/ServerStream" => {
                Box::pin(async move { Ok(grpc.server_streaming(ServerStreamM, req).await) })
            }
            "/pkg.Svc/ClientStream" => {
                Box::pin(async move { Ok(grpc.client_streaming(ClientStreamM, req).await) })
            }
            "/pkg.Svc/Bidi" => Box::pin(async move { Ok(grpc.streaming(BidiM, req).await) }),
            _ => Box::pin(async { Ok(tonic::Status::unimplemented("no such method").into_http()) }),
        }
    }
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

/// A tonic server on the oracle runtime; its address.
fn tonic_server(oracle: &tokio::runtime::Runtime) -> std::net::SocketAddr {
    oracle.block_on(async {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let incoming = tonic::transport::server::TcpIncoming::from(l);
        tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(Svc)
                .serve_with_incoming(incoming),
        );
        addr
    })
}

fn with_server<F, Fut>(f: F)
where
    F: FnOnce(Channel) -> Fut,
    Fut: Future<Output = ()>,
{
    let (ours, oracle) = runtimes();
    let addr = tonic_server(&oracle);
    let channel = Channel::new(&format!("http://{addr}"), ChannelOptions::default()).unwrap();
    ours.block_on(async move { rusty_tokio::time::timeout(FIVE * 4, f(channel)).await })
        .unwrap();
}

#[test]
fn a_unary_call_and_its_metadata() {
    with_server(|c| async move {
        let mut req = Request::new(blob(7, 1000));
        req.metadata.insert("x-in", "1").unwrap();
        let r = c.unary::<Blob, Blob>("/pkg.Svc/Unary", req).await.unwrap();
        assert_eq!(r.message.n, 8);
        assert_eq!(r.message.data, blob(7, 1000).data);
        assert_eq!(r.metadata.get("x-reply"), Some("yes"));
    });
}

#[test]
fn an_error_with_metadata_and_an_unimplemented_method() {
    with_server(|c| async move {
        let e = c
            .unary::<Blob, Blob>("/pkg.Svc/Unary", Request::new(blob(99, 0)))
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::NotFound);
        assert_eq!(e.message(), "gone");
        assert_eq!(e.metadata().get("x-why"), Some("missing"));

        let e = c
            .unary::<Blob, Blob>("/pkg.Svc/Nope", Request::new(blob(1, 0)))
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::Unimplemented);
    });
}

#[test]
fn a_server_stream() {
    with_server(|c| async move {
        let mut s = c
            .server_streaming::<Blob, Blob>("/pkg.Svc/ServerStream", Request::new(blob(5, 0)))
            .await
            .unwrap();
        for i in 0..5 {
            assert_eq!(s.message().await.unwrap().unwrap().n, i);
        }
        assert_eq!(s.message().await.unwrap(), None);
        assert!(s.trailers().is_some());
    });
}

#[test]
fn a_client_stream() {
    with_server(|c| async move {
        let (mut tx, reply) = c
            .client_streaming::<Blob, Blob>("/pkg.Svc/ClientStream", Request::new(()))
            .await
            .unwrap();
        for i in 1..=10 {
            tx.send(&blob(i, 100)).await.unwrap();
        }
        tx.finish().await.unwrap();
        assert_eq!(reply.response().await.unwrap().message.n, 55);
    });
}

#[test]
fn a_bidirectional_stream() {
    with_server(|c| async move {
        let (mut tx, mut rx) = c
            .bidirectional::<Blob, Blob>("/pkg.Svc/Bidi", Request::new(()))
            .await
            .unwrap();
        for i in 0..20 {
            tx.send(&blob(i, 10)).await.unwrap();
            assert_eq!(rx.message().await.unwrap().unwrap().n, i + 100);
        }
        tx.finish().await.unwrap();
        assert_eq!(rx.message().await.unwrap(), None);
    });
}
