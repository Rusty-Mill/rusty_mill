//! Shared helpers for the client tests: a message type, and a small gRPC server built from the
//! core's own pieces over `rusty_hyper::serve`, so each test scripts exactly what the server does.
#![allow(dead_code, clippy::unwrap_used)]

use std::time::Duration;

use rusty_h2::driver::{Body, RecvStream, SendStream};
use rusty_hyper::{serve, HeaderField, Request, Respond, ServeOptions, Server};
use rusty_prost::{Field, Message, Result as PResult, Writer};
use rusty_tokio::io::TcpListener;
use rusty_tonic::client::{Channel, ChannelOptions};
use rusty_tonic::{FrameDecoder, MetadataMap, RequestHead, ResponseHead, Status};

/// `message Blob { bytes data = 1; uint32 n = 2; }`
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Blob {
    pub data: Vec<u8>,
    pub n: u32,
}

impl Message for Blob {
    fn encode(&self, w: &mut Writer) {
        if !self.data.is_empty() {
            w.bytes(1, &self.data);
        }
        if self.n != 0 {
            w.uint32(2, self.n);
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> PResult<()> {
        match f.number {
            1 => self.data = f.bytes()?.to_vec(),
            2 => self.n = f.uint32()?,
            _ => {}
        }
        Ok(())
    }
}

/// A blob numbered `n` with `len` patterned bytes.
pub fn blob(n: u32, len: usize) -> Blob {
    Blob {
        data: (0..len).map(|i| (i.wrapping_mul(31) % 251) as u8).collect(),
        n,
    }
}

pub fn listener() -> TcpListener {
    let std = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    TcpListener::from_std(std).unwrap()
}

/// Serves `handler` as a gRPC server on a fresh port.
pub fn start<F, Fut>(handler: F) -> Server
where
    F: Fn(Request, Respond) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    serve(listener(), None, ServeOptions::default(), handler).unwrap()
}

/// A channel to `server` with default options.
pub fn channel(server: &Server) -> Channel {
    Channel::new(
        &format!("http://{}", server.local_addr()),
        ChannelOptions::default(),
    )
    .unwrap()
}

pub fn fields(pairs: &[(String, Vec<u8>)]) -> Vec<HeaderField> {
    pairs
        .iter()
        .map(|(n, v)| HeaderField::new(n.as_str(), v.as_slice()))
        .collect()
}

/// A request header list as the borrowed pairs the core's parsers take.
pub fn pairs(headers: &[HeaderField]) -> Vec<(&str, &[u8])> {
    headers
        .iter()
        .map(|h| (std::str::from_utf8(&h.name).unwrap(), h.value.as_slice()))
        .collect()
}

pub fn header<'a>(headers: &'a [HeaderField], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name == name.as_bytes())
        .map(|h| std::str::from_utf8(&h.value).unwrap())
}

/// What the server read of a request.
pub struct Incoming {
    pub raw: Vec<HeaderField>,
    pub head: RequestHead,
    /// The whole messages, undecoded.
    pub messages: Vec<Vec<u8>>,
}

impl Incoming {
    pub fn blobs(&self) -> Vec<Blob> {
        self.messages
            .iter()
            .map(|m| Blob::decode(m).unwrap())
            .collect()
    }
}

/// Reads a request to its end.
pub async fn read_call(req: &mut Request) -> Incoming {
    let raw = std::mem::take(&mut req.headers);
    let head = RequestHead::from_headers(pairs(&raw)).unwrap();
    let mut decoder = FrameDecoder::new(usize::MAX);
    let mut messages = Vec::new();
    while let Some(piece) = req.body.next().await {
        if let Ok(Body::Data(bytes)) = piece {
            decoder.push(&bytes);
            while let Some(m) = decoder.next_message().unwrap() {
                messages.push(m);
            }
        }
    }
    Incoming {
        raw,
        head,
        messages,
    }
}

/// The next whole message of a request, or `None` once its body ended or was reset.
pub async fn next_message(body: &mut RecvStream, decoder: &mut FrameDecoder) -> Option<Vec<u8>> {
    loop {
        if let Some(m) = decoder.next_message().unwrap() {
            return Some(m);
        }
        match body.next().await {
            Some(Ok(Body::Data(bytes))) => decoder.push(&bytes),
            Some(Ok(Body::Trailers(_))) => {}
            Some(Err(_)) | None => return None,
        }
    }
}

/// Starts a response: `:status 200`, the content type and `metadata`.
pub async fn start_response(respond: Respond, metadata: &MetadataMap) -> SendStream {
    let head = ResponseHead {
        metadata: metadata.clone(),
        metadata_dropped: 0,
    };
    respond
        .send_response(fields(&head.to_headers()), false)
        .await
        .unwrap()
}

pub async fn send_message(send: &mut SendStream, message: &Blob) {
    let frame = rusty_tonic::encode_message(message).unwrap();
    send.send_data(frame, false).await.unwrap();
}

/// Ends a response with `status` as its trailers.
pub async fn finish_response(mut send: SendStream, status: &Status) {
    send.send_trailers(fields(&status.to_trailers()))
        .await
        .unwrap();
}

/// A whole response: `messages`, then `status`.
pub async fn reply(respond: Respond, messages: &[Blob], status: &Status) {
    let mut send = start_response(respond, &MetadataMap::new()).await;
    for m in messages {
        send_message(&mut send, m).await;
    }
    finish_response(send, status).await;
}

/// A "trailers-only" response: the call is over in its first headers.
pub async fn trailers_only(respond: Respond, status: &Status) {
    let mut headers = vec![
        (":status".to_string(), b"200".to_vec()),
        ("content-type".to_string(), b"application/grpc".to_vec()),
    ];
    headers.extend(status.to_trailers());
    respond.send_response(fields(&headers), true).await.unwrap();
}

/// Answers every call with `blob`, echoing the request's first message.
pub async fn echo(mut req: Request, respond: Respond) {
    let call = read_call(&mut req).await;
    let blobs = call.blobs();
    reply(respond, &blobs, &Status::ok()).await;
}

pub const SHORT: Duration = Duration::from_millis(150);
pub const FIVE: Duration = Duration::from_secs(5);
