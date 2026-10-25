//! The upstream half of a proxied exchange, on `rusty_request`.
//!
//! A request goes up whole: the gateway reads its body, within its cap,
//! before it gets here, so every attempt can be replayed. The response comes
//! back streamed, a chunk at a time as the client takes it, because a proxy
//! that buffers turns a download into a memory limit. Once its body has been
//! read to the end, the connection goes back to `rusty_request`'s pool.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use http::{HeaderName, HeaderValue, Request, Response, StatusCode, header};
use rusty_request::{Client, StreamingResponse};

/// Why an attempt got no response.
pub(crate) type SendError = rusty_request::Error;

/// Sends `request` to the URI it names and returns once the response head
/// has arrived.
///
/// `Host` is not copied: `rusty_request` writes it from the URI, which the
/// proxy has already pointed at the upstream. The `Content-Length` it
/// writes is the body's own. A field the client sent replaces the one
/// `rusty_request` would add by default (its `User-Agent`), and every value
/// of a repeated field is kept; a request without a `User-Agent` goes up
/// with `rusty_request`'s.
pub(crate) async fn send(
    client: &Client,
    request: &Request<Bytes>,
) -> Result<Response<Body>, SendError> {
    let method = rusty_request::Method::parse(request.method().as_str());
    let mut builder = client.request(method, &request.uri().to_string())?;
    let headers = request.headers();
    for name in headers.keys() {
        if name == header::HOST || name == header::CONTENT_LENGTH {
            continue;
        }
        let mut replaced = false;
        for value in headers.get_all(name) {
            let Ok(value) = value.to_str() else {
                rusty_log::debug!(header = %name, "dropping a request header that is not text");
                continue;
            };
            builder = if replaced {
                builder.header_append(name.as_str(), value)?
            } else {
                builder.header(name.as_str(), value)?
            };
            replaced = true;
        }
    }
    let response = builder
        .body(request.body().to_vec())
        .send_streaming()
        .await?;
    Ok(into_http(response))
}

/// The response head as `http` types, with the body still to be read.
fn into_http(response: StreamingResponse) -> Response<Body> {
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut headers = http::HeaderMap::new();
    for (name, value) in response.headers().iter() {
        match (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            (Ok(name), Ok(value)) => {
                headers.append(name, value);
            }
            _ => rusty_log::debug!(header = %name, "dropping an upstream header http cannot hold"),
        }
    }
    let mut out = Response::new(Body::new(response));
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    out
}

/// The next chunk, with the response handed back for the one after.
type Reading = Pin<
    Box<dyn Future<Output = (StreamingResponse, rusty_request::Result<Option<Vec<u8>>>)> + Send>,
>;

enum State {
    /// Waiting to be asked for the next chunk.
    Idle(Box<StreamingResponse>),
    /// A chunk is on its way.
    Reading(Reading),
    /// Read to the end, or failed.
    Done,
}

/// An upstream response body, read as the client takes it.
pub struct Body(State);

impl Body {
    fn new(response: StreamingResponse) -> Self {
        Body(State::Idle(Box::new(response)))
    }
}

impl http_body::Body for Body {
    type Data = Bytes;
    type Error = rusty_request::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
        let state = &mut self.get_mut().0;
        loop {
            match std::mem::replace(state, State::Done) {
                State::Done => return Poll::Ready(None),
                State::Idle(mut response) => {
                    *state = State::Reading(Box::pin(async move {
                        let chunk = response.chunk().await;
                        (*response, chunk)
                    }));
                }
                State::Reading(mut reading) => {
                    let Poll::Ready((response, chunk)) = reading.as_mut().poll(cx) else {
                        *state = State::Reading(reading);
                        return Poll::Pending;
                    };
                    return match chunk {
                        Ok(Some(data)) => {
                            *state = State::Idle(Box::new(response));
                            Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(data)))))
                        }
                        // The response, dropped here, has already pooled
                        // its connection.
                        Ok(None) => Poll::Ready(None),
                        Err(err) => Poll::Ready(Some(Err(err))),
                    };
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self.0, State::Done)
    }
}
