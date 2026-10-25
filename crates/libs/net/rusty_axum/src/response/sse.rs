//! Server-sent events: a stream of [`Event`]s as a `text/event-stream`.
//!
//! Building one is runtime-free. [`Sse::keep_alive`] adds a timer, so a
//! stream with keep-alive must be polled inside a `rusty_tokio` runtime, as
//! the server does.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use rusty_http::StatusCode;
use rusty_tokio::time::{sleep, Sleep};

use super::{IntoResponse, Response};
use crate::body::{Body, BoxError};

/// One event. Built like axum's: `Event::default().event("token").data(s)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Event {
    event: Option<String>,
    id: Option<String>,
    data: Option<String>,
    comment: Option<String>,
}

impl Event {
    /// The event name (`event:`). Line breaks are removed: a name is one
    /// line on the wire.
    pub fn event(mut self, name: impl AsRef<str>) -> Self {
        self.event = Some(one_line(name.as_ref()));
        self
    }

    /// The event id (`id:`), which a client resends as `Last-Event-ID`.
    /// Line breaks are removed, as for [`Event::event`].
    pub fn id(mut self, id: impl AsRef<str>) -> Self {
        self.id = Some(one_line(id.as_ref()));
        self
    }

    /// The data. Several lines become several `data:` lines, which the
    /// client joins back with `\n`.
    pub fn data(mut self, data: impl Into<String>) -> Self {
        self.data = Some(data.into());
        self
    }

    /// The data as JSON.
    ///
    /// # Errors
    ///
    /// When `value` does not serialize.
    pub fn json_data<T: serde::Serialize>(self, value: &T) -> serde_json::Result<Self> {
        Ok(self.data(serde_json::to_string(value)?))
    }

    /// A comment line (`:`), ignored by clients.
    pub fn comment(mut self, text: impl AsRef<str>) -> Self {
        self.comment = Some(one_line(text.as_ref()));
        self
    }

    /// The event on the wire, ending with the blank line.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        if let Some(comment) = &self.comment {
            out.push_str(&format!(":{comment}\n"));
        }
        if let Some(event) = &self.event {
            out.push_str(&format!("event: {event}\n"));
        }
        if let Some(id) = &self.id {
            out.push_str(&format!("id: {id}\n"));
        }
        if let Some(data) = &self.data {
            for line in data_lines(data) {
                out.push_str(&format!("data: {line}\n"));
            }
        }
        out.push('\n');
        out.into_bytes()
    }
}

/// The data's logical lines. SSE ends a line at CRLF, a lone CR or a lone
/// LF, so all three must start a new `data:` line: an unprefixed CR would
/// let data text such as `event: x` become a field of its own.
fn data_lines(data: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(data);
    std::iter::from_fn(move || {
        let text = rest?;
        match text.find(['\r', '\n']) {
            None => {
                rest = None;
                Some(text)
            }
            Some(i) => {
                let skip = if text[i..].starts_with("\r\n") { 2 } else { 1 };
                rest = Some(&text[i + skip..]);
                Some(&text[..i])
            }
        }
    })
}

fn one_line(text: &str) -> String {
    text.chars().filter(|c| *c != '\r' && *c != '\n').collect()
}

/// A `text/event-stream` response from a stream of events. An `Err` item
/// ends the stream.
pub struct Sse<S> {
    stream: S,
    keep_alive: Option<KeepAlive>,
}

impl<S> Sse<S> {
    /// Wraps `stream`.
    pub fn new(stream: S) -> Self {
        Sse {
            stream,
            keep_alive: None,
        }
    }

    /// Sends `keep_alive`'s comment whenever the stream has been quiet for
    /// its interval, so proxies and clients do not time an idle stream out.
    pub fn keep_alive(mut self, keep_alive: KeepAlive) -> Self {
        self.keep_alive = Some(keep_alive);
        self
    }
}

/// How [`Sse::keep_alive`] keeps a quiet stream open: a comment after each
/// interval without an event. As axum's, 15 s and an empty comment by
/// default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepAlive {
    interval: Duration,
    comment: Vec<u8>,
}

impl Default for KeepAlive {
    fn default() -> Self {
        KeepAlive {
            interval: Duration::from_secs(15),
            comment: Event::default().comment("").to_bytes(),
        }
    }
}

impl KeepAlive {
    /// The default keep-alive.
    pub fn new() -> Self {
        KeepAlive::default()
    }

    /// The quiet time before each comment.
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// The comment's text, one line.
    pub fn text(mut self, text: impl AsRef<str>) -> Self {
        self.comment = Event::default().comment(text).to_bytes();
        self
    }
}

impl<S, E> IntoResponse for Sse<S>
where
    S: Stream<Item = Result<Event, E>> + Send + 'static,
    E: Into<BoxError>,
{
    fn into_response(self) -> Response {
        let encode = Encode {
            stream: Box::pin(self.stream),
            keep_alive: self.keep_alive,
            timer: None,
        };
        Response::new(StatusCode::OK, Body::from_stream(encode))
            .with_header("content-type", "text/event-stream")
            .with_header("cache-control", "no-cache")
    }
}

/// Encodes each event as it arrives, and sends the keep-alive comment when
/// none has for an interval. The timer starts on the first quiet poll, so a
/// stream that is never quiet never needs a runtime.
struct Encode<S> {
    stream: Pin<Box<S>>,
    keep_alive: Option<KeepAlive>,
    timer: Option<Sleep>,
}

impl<S, E> Stream for Encode<S>
where
    S: Stream<Item = Result<Event, E>>,
    E: Into<BoxError>,
{
    type Item = Result<Vec<u8>, BoxError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Poll::Ready(item) = this.stream.as_mut().poll_next(cx) {
            this.timer = None;
            return Poll::Ready(item.map(|r| r.map(|e| e.to_bytes()).map_err(Into::into)));
        }
        let Some(keep_alive) = &this.keep_alive else {
            return Poll::Pending;
        };
        let timer = this.timer.get_or_insert_with(|| sleep(keep_alive.interval));
        if Pin::new(timer).poll(cx).is_pending() {
            return Poll::Pending;
        }
        this.timer = None;
        Poll::Ready(Some(Ok(keep_alive.comment.clone())))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn encodes_name_id_and_multiline_data() {
        let e = Event::default().event("token").id("7").data("a\nb");
        assert_eq!(e.to_bytes(), b"event: token\nid: 7\ndata: a\ndata: b\n\n");
    }

    #[test]
    fn every_line_break_starts_a_data_line() {
        let e = Event::default().data("a\r\nb\rc\nd");
        assert_eq!(e.to_bytes(), b"data: a\ndata: b\ndata: c\ndata: d\n\n");
        let e = Event::default().data("x\r\r");
        assert_eq!(e.to_bytes(), b"data: x\ndata: \ndata: \n\n");
    }

    #[test]
    fn field_text_in_data_stays_data() {
        let e = Event::default().data("safe\revent: forged\rid: 9\rretry: 1\r\rdata: injected");
        assert_eq!(
            e.to_bytes(),
            b"data: safe\ndata: event: forged\ndata: id: 9\ndata: retry: 1\ndata: \ndata: data: injected\n\n"
        );
    }

    #[test]
    fn strips_line_breaks_from_one_line_fields() {
        let e = Event::default().event("x\r\ny").comment("c\nd");
        assert_eq!(e.to_bytes(), b":cd\nevent: xy\n\n");
    }

    #[test]
    fn json_data_is_one_line() {
        let e = Event::default()
            .json_data(&serde_json::json!({"a": 1}))
            .map(|e| e.to_bytes());
        assert_eq!(e.ok(), Some(b"data: {\"a\":1}\n\n".to_vec()));
    }
}
