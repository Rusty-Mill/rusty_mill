//! Server-sent events: a stream of [`Event`]s as a `text/event-stream`.
//!
//! Keep-alive comments need a timer, so they belong to the server driver,
//! not this runtime-free module; no keep-alive is sent yet.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use rusty_http::StatusCode;

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
pub struct Sse<S>(S);

impl<S> Sse<S> {
    /// Wraps `stream`.
    pub fn new(stream: S) -> Self {
        Sse(stream)
    }
}

impl<S, E> IntoResponse for Sse<S>
where
    S: Stream<Item = Result<Event, E>> + Send + 'static,
    E: Into<BoxError>,
{
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK, Body::from_stream(Encode(Box::pin(self.0))))
            .with_header("content-type", "text/event-stream")
            .with_header("cache-control", "no-cache")
    }
}

/// Encodes each event as it arrives.
struct Encode<S>(Pin<Box<S>>);

impl<S, E> Stream for Encode<S>
where
    S: Stream<Item = Result<Event, E>>,
    E: Into<BoxError>,
{
    type Item = Result<Vec<u8>, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0
            .as_mut()
            .poll_next(cx)
            .map(|item| item.map(|r| r.map(|e| e.to_bytes()).map_err(Into::into)))
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
