//! Helpers for the server tests: serving a router, and a bare HTTP/2 client that sends exactly
//! the bytes a test wants (so a misbehaving client can be written).
#![allow(dead_code, clippy::unwrap_used)]

use std::net::SocketAddr;

use rusty_hyper::{connect, ConnectOptions, H2Error, HeaderField, ServeOptions, Server};
use rusty_tonic::server::Router;
use rusty_tonic::{RequestHead, Status};

use crate::client_common::{fields, listener, pairs};

/// Serves `router` on a fresh port.
pub fn serve(router: Router) -> Server {
    router
        .serve(listener(), None, ServeOptions::default())
        .unwrap()
}

/// The standard header list of a gRPC call to `path`.
pub fn grpc_headers(path: &str) -> Vec<(String, Vec<u8>)> {
    RequestHead::new(path, "localhost").to_headers()
}

/// A length-prefixed message frame around `payload`.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0];
    out.extend((payload.len() as u32).to_be_bytes());
    out.extend(payload);
    out
}

/// What a bare client got back.
pub struct Raw {
    pub headers: Vec<HeaderField>,
    pub data: Vec<u8>,
    pub trailers: Option<Vec<HeaderField>>,
}

impl Raw {
    /// The call's status, from the trailers, or from the headers of a "trailers-only" response.
    pub fn status(&self) -> Status {
        let source = self.trailers.as_ref().unwrap_or(&self.headers);
        Status::from_headers(pairs(source)).expect("a grpc-status")
    }

    /// Whether the response was a single header block that ended the call.
    pub fn trailers_only(&self) -> bool {
        self.trailers.is_none() && self.data.is_empty()
    }
}

/// Opens a connection to `addr`, sends `headers`, then each of `chunks` as DATA (ending the
/// request after the last when `end` is true), and reads the whole response.
pub async fn raw(
    addr: SocketAddr,
    headers: Vec<(String, Vec<u8>)>,
    chunks: &[Vec<u8>],
    end: bool,
) -> Result<Raw, H2Error> {
    let conn = connect(&format!("http://{addr}"), ConnectOptions::default())
        .await
        .unwrap();
    let (mut send, response) = conn
        .client()
        .send_request(fields(&headers), chunks.is_empty() && end)
        .await?;
    for (i, chunk) in chunks.iter().enumerate() {
        send.send_data(chunk.clone(), end && i + 1 == chunks.len())
            .await?;
    }
    let mut response = response.await?;
    let (mut data, mut trailers) = (Vec::new(), None);
    while let Some(piece) = response.body.next().await {
        match piece? {
            rusty_hyper::Body::Data(d) => data.extend(d),
            rusty_hyper::Body::Trailers(t) => trailers = Some(t),
        }
    }
    drop(send);
    Ok(Raw {
        headers: response.headers,
        data,
        trailers,
    })
}
