//! `WireCodec` on a real `tonic` server: the request is read and the reply written with
//! `rusty_prost`, and a body that is not the message is an `Internal` status, not a panic.
#![allow(clippy::unwrap_used)]

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use rusty_prost::message;
use rusty_tonic::tonic_codec::WireCodec;
use rusty_tonic::{decode_message, encode_frame, encode_message, FrameDecoder};
use tonic::server::{Grpc, UnaryService};
use tonic::{Request, Response, Status};

message! {
    /// A message with a string and a number.
    Ping {
        1 text: String = string,
        2 n: i32 = int32,
    }
}

/// Answers a `Ping` with the text upper-cased and the number doubled.
struct Shout;

impl UnaryService<Ping> for Shout {
    type Response = Ping;
    type Future = std::future::Ready<Result<Response<Ping>, Status>>;

    fn call(&mut self, request: Request<Ping>) -> Self::Future {
        let ping = request.into_inner();
        std::future::ready(Ok(Response::new(Ping {
            text: ping.text.to_uppercase(),
            n: ping.n * 2,
        })))
    }
}

async fn call(body: Vec<u8>) -> http::Response<tonic::body::Body> {
    let request = http::Request::post("/pkg.Svc/Shout")
        .header("content-type", "application/grpc")
        .body(Full::new(Bytes::from(body)))
        .unwrap();
    Grpc::new(WireCodec::<Ping, Ping>::default())
        .unary(Shout, request)
        .await
}

#[rusty_tokio::test]
async fn a_message_is_read_and_answered_with_rusty_prost() {
    let ping = Ping {
        text: "hi".into(),
        n: 21,
    };
    let response = call(encode_message(&ping).unwrap()).await;
    let collected = response.into_body().collect().await.unwrap();
    let trailers = collected.trailers().cloned().unwrap();
    let body = collected.to_bytes();
    let mut frames = FrameDecoder::new(1024);
    frames.push(&body);
    let reply: Ping = decode_message(&frames.next_message().unwrap().unwrap()).unwrap();
    assert_eq!(reply.text, "HI");
    assert_eq!(reply.n, 42);
    assert_eq!(trailers["grpc-status"], "0");
}

#[rusty_tokio::test]
async fn a_body_that_is_not_the_message_is_an_internal_status() {
    // A valid frame around bytes that are not a `Ping` (a truncated length-delimited field).
    let response = call(encode_frame(&[0x0a, 0x09, b'x']).unwrap()).await;
    // A refusal before any message is trailers-only: the status is in the headers.
    let status = response.headers()["grpc-status"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(status, "13", "INTERNAL");
}
