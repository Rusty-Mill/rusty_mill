//! Between `rusty_axum`'s request and response and the `http` types the data
//! plane is written in.
//!
//! The server reads each request body whole, within a cap, before routing, so
//! a request crosses as `http::Request<Bytes>`. A response crosses back with
//! its body whole when it is small and its size is known, so it keeps its
//! `Content-Length`; anything else streams.

use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body::Body as _;
use http_body_util::BodyExt as _;
use rusty_axum::Stream;

use crate::body::{Body, BoxError};

/// The largest response body sent whole, with a `Content-Length`. A larger
/// one, or one of unknown size, is streamed: buffering a large download would
/// hold all of it in memory for no gain.
const MAX_WHOLE_RESPONSE: u64 = 1 << 20;

/// The request as the data plane reads it.
///
/// # Errors
///
/// A `400` when the method, target or a header is not one `http` accepts.
pub fn request(
    mut request: rusty_axum::Request,
) -> Result<http::Request<Bytes>, rusty_axum::Response> {
    let bad = |what: &str| {
        rusty_axum::Response::new(
            rusty_axum::StatusCode::BAD_REQUEST,
            format!("the request {what} is not valid"),
        )
    };
    let method = http::Method::from_bytes(request.method().as_str().as_bytes())
        .map_err(|_| bad("method"))?;
    let uri = request
        .target()
        .parse::<http::Uri>()
        .map_err(|_| bad("target"))?;
    let mut headers = http::HeaderMap::with_capacity(request.headers().len());
    for (name, value) in request.headers().iter() {
        let name = http::HeaderName::from_bytes(name.as_bytes()).map_err(|_| bad("header name"))?;
        let value = http::HeaderValue::from_str(value).map_err(|_| bad("header value"))?;
        headers.append(name, value);
    }
    let body = Bytes::from(request.take_body());
    let mut out = http::Request::new(body);
    *out.method_mut() = method;
    *out.uri_mut() = uri;
    *out.headers_mut() = headers;
    Ok(out)
}

/// The response as the server sends it.
///
/// A header value that is not text cannot be sent by the server and is
/// dropped, as is anything it frames itself (`Content-Length`,
/// `Transfer-Encoding`, `Connection`), which it replaces.
pub async fn response(response: http::Response<Body>) -> rusty_axum::Response {
    let (parts, body) = response.into_parts();
    let mut headers = rusty_axum::HeaderMap::new();
    for (name, value) in &parts.headers {
        let Ok(value) = value.to_str() else {
            rusty_log::debug!(header = %name, "dropping a response header that is not text");
            continue;
        };
        if let Err(err) = headers.append(name.as_str(), value) {
            rusty_log::debug!(header = %name, %err, "dropping a response header the server refuses");
        }
    }
    let status = rusty_axum::StatusCode::from_u16(parts.status.as_u16());
    let body = match body.size_hint().exact() {
        Some(len) if len <= MAX_WHOLE_RESPONSE => match body.collect().await {
            Ok(collected) => rusty_axum::Body::from(collected.to_bytes().to_vec()),
            // The body failed before it was sent; the client sees it cut off.
            Err(err) => {
                rusty_log::debug!(%err, "a response body failed");
                rusty_axum::Body::from_stream(Failed(Some(err)))
            }
        },
        _ => rusty_axum::Body::from_stream(Frames(body)),
    };
    rusty_axum::Response {
        status,
        headers,
        body,
    }
}

/// A body's data frames as a stream of chunks. Trailers are dropped: the
/// server does not send them.
struct Frames(Body);

impl Stream for Frames {
    type Item = Result<Vec<u8>, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let frame = match Pin::new(&mut self.0).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => frame,
                Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            };
            if let Ok(data) = frame.into_data()
                && !data.is_empty()
            {
                return Poll::Ready(Some(Ok(data.to_vec())));
            }
        }
    }
}

/// A body that has already failed: one error, then the end.
struct Failed(Option<BoxError>);

impl Stream for Failed {
    type Item = Result<Vec<u8>, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.0.take().map(Err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use http_body_util::StreamBody;

    fn request_to(target: &str) -> rusty_axum::Request {
        rusty_axum::Request::new(rusty_axum::Method::Post, target)
            .with_header("host", "gateway.test")
            .and_then(|r| r.with_header("x-repeated", "one"))
            .and_then(|r| r.with_header("x-repeated", "two"))
            .expect("valid headers")
            .with_body(b"payload".to_vec())
    }

    #[test]
    fn a_request_keeps_its_method_target_headers_and_body() {
        let request = request(request_to("/mcp?session=1")).expect("valid");
        assert_eq!(request.method(), http::Method::POST);
        assert_eq!(request.uri().path(), "/mcp");
        assert_eq!(request.uri().query(), Some("session=1"));
        assert_eq!(request.headers()["host"], "gateway.test");
        let repeated: Vec<_> = request.headers().get_all("x-repeated").iter().collect();
        assert_eq!(repeated, ["one", "two"]);
        assert_eq!(request.body().as_ref(), b"payload");
    }

    #[test]
    fn a_target_http_cannot_parse_is_a_400() {
        let refused = request(rusty_axum::Request::new(rusty_axum::Method::Get, "/a b"))
            .expect_err("a space is not a valid target");
        assert_eq!(refused.status.as_u16(), 400);
    }

    #[rusty_tokio::test]
    async fn a_small_whole_body_is_sent_whole_with_its_headers() {
        let mut response = http::Response::new(Body::from(Bytes::from_static(b"hello")));
        *response.status_mut() = http::StatusCode::CREATED;
        response
            .headers_mut()
            .append("set-cookie", http::HeaderValue::from_static("a=1"));
        response
            .headers_mut()
            .append("set-cookie", http::HeaderValue::from_static("b=2"));
        response.headers_mut().append(
            "x-binary",
            http::HeaderValue::from_bytes(b"\xff").expect("obs-text is a valid value"),
        );

        let sent = super::response(response).await;
        assert_eq!(sent.status.as_u16(), 201);
        assert_eq!(sent.body.as_bytes(), Some(&b"hello"[..]));
        let cookies: Vec<_> = sent.headers.get_all("set-cookie").collect();
        assert_eq!(cookies, ["a=1", "b=2"]);
        assert!(sent.headers.get("x-binary").is_none());
    }

    #[rusty_tokio::test]
    async fn a_body_of_unknown_size_streams() {
        let chunks = ["one", "two"].map(|chunk| {
            Ok::<_, std::convert::Infallible>(http_body::Frame::data(Bytes::from_static(
                chunk.as_bytes(),
            )))
        });
        let body = Body::new(StreamBody::new(Iter(chunks.into_iter())));
        let sent = super::response(http::Response::new(body)).await;
        assert!(sent.body.as_bytes().is_none(), "a stream is not buffered");
        assert_eq!(sent.body.collect().await.expect("whole"), b"onetwo");
    }

    #[rusty_tokio::test]
    async fn a_large_body_streams_even_when_its_size_is_known() {
        let large = Bytes::from(vec![b'x'; (MAX_WHOLE_RESPONSE + 1) as usize]);
        let sent = super::response(http::Response::new(Body::from(large.clone()))).await;
        assert!(sent.body.as_bytes().is_none());
        assert_eq!(sent.body.collect().await.expect("whole"), large.to_vec());
    }

    /// An iterator as a stream, ready at every poll.
    struct Iter<I>(I);

    impl<I: Iterator + Unpin> Stream for Iter<I> {
        type Item = I::Item;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<I::Item>> {
            Poll::Ready(self.0.next())
        }
    }
}
