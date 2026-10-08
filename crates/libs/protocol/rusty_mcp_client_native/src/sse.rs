//! A server-sent-events parser (the WHATWG "Server-sent events" line
//! protocol), pure: bytes in, events out. The Streamable HTTP transport feeds
//! it the chunks of a `text/event-stream` body.
//!
//! Handles `\n`, `\r\n` and `\r` line ends (also split across chunks), a
//! leading byte-order mark, comment lines, multi-line `data`, fields without a
//! colon, and chunk boundaries anywhere, including inside a multi-byte
//! character. `id` and `retry` persist across events, as the spec says.

/// One dispatched event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event` field; `None` means the default type, `message`.
    pub event: Option<String>,
    /// The `data` lines joined with `\n`.
    pub data: String,
    /// The last `id` seen on this stream (it persists across events).
    pub id: Option<String>,
}

/// Incremental parser.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    started: bool,
    /// A `\r` ended the last line, so a following `\n` belongs to it.
    after_cr: bool,
    event: Option<String>,
    data: Vec<String>,
    has_data: bool,
    last_id: Option<String>,
    retry: Option<u64>,
}

impl SseParser {
    /// A parser at the start of a stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// The last `id` seen: what `Last-Event-ID` should carry on reconnect.
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }

    /// The last `retry` value (milliseconds) the server asked for.
    pub fn retry_ms(&self) -> Option<u64> {
        self.retry
    }

    /// Feed the next bytes of the stream; returns the events they complete.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while i < self.buffer.len() {
            let b = self.buffer[i];
            if self.after_cr && i == start && b == b'\n' {
                // The `\n` of a `\r\n` whose `\r` ended the previous line.
                self.after_cr = false;
                start = i + 1;
                i += 1;
                continue;
            }
            self.after_cr = false;
            if b == b'\n' || b == b'\r' {
                let line = self.buffer[start..i].to_vec();
                if b == b'\r' {
                    if i + 1 < self.buffer.len() {
                        if self.buffer[i + 1] == b'\n' {
                            i += 1;
                        }
                    } else {
                        self.after_cr = true;
                    }
                }
                start = i + 1;
                if let Some(event) = self.line(&line) {
                    events.push(event);
                }
            }
            i += 1;
        }
        self.buffer.drain(..start);
        events
    }

    fn line(&mut self, raw: &[u8]) -> Option<SseEvent> {
        let mut text = String::from_utf8_lossy(raw).into_owned();
        if !self.started {
            self.started = true;
            if let Some(rest) = text.strip_prefix('\u{feff}') {
                text = rest.to_owned();
            }
        }
        if text.is_empty() {
            return self.dispatch();
        }
        if text.starts_with(':') {
            return None;
        }
        let (field, value) = match text.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (text.as_str(), ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => {
                self.data.push(value.to_owned());
                self.has_data = true;
            }
            "id" if !value.contains('\0') => self.last_id = Some(value.to_owned()),
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                self.retry = value.parse().ok();
            }
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take();
        let has_data = std::mem::take(&mut self.has_data);
        let data = std::mem::take(&mut self.data).join("\n");
        has_data.then(|| SseEvent {
            event,
            data,
            id: self.last_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(input: &[u8]) -> Vec<SseEvent> {
        SseParser::new().feed(input)
    }

    fn ev(data: &str) -> SseEvent {
        SseEvent {
            data: data.to_owned(),
            ..SseEvent::default()
        }
    }

    #[test]
    fn a_simple_event() {
        assert_eq!(all(b"data: hello\n\n"), [ev("hello")]);
    }

    #[test]
    fn the_server_framing_used_by_rusty_mcp_server() {
        let got = all(b"id: 7\nevent: message\ndata: {\"a\":1}\n\n");
        assert_eq!(
            got,
            [SseEvent {
                event: Some("message".into()),
                data: "{\"a\":1}".into(),
                id: Some("7".into()),
            }]
        );
    }

    #[test]
    fn multiline_data_joins_with_newlines() {
        assert_eq!(all(b"data: a\ndata: b\ndata\n\n"), [ev("a\nb\n")]);
    }

    #[test]
    fn all_three_line_endings_and_a_split_crlf() {
        for input in [&b"data: x\n\n"[..], b"data: x\r\n\r\n", b"data: x\r\r"] {
            assert_eq!(all(input), [ev("x")], "{input:?}");
        }
        // The CR and the LF of one line ending arrive in different chunks.
        let mut p = SseParser::new();
        let mut got = p.feed(b"data: x\r");
        got.extend(p.feed(b"\n\r"));
        got.extend(p.feed(b"\n"));
        assert_eq!(got, [ev("x")]);
    }

    #[test]
    fn comments_and_unknown_fields_are_ignored_and_a_keepalive_is_not_an_event() {
        assert_eq!(all(b": ping\n\n"), []);
        assert_eq!(all(b": hi\nfoo: bar\ndata: y\n\n"), [ev("y")]);
    }

    #[test]
    fn an_event_without_data_is_not_dispatched() {
        assert_eq!(all(b"event: x\n\n"), []);
    }

    #[test]
    fn only_one_leading_space_is_stripped() {
        assert_eq!(all(b"data:  two\n\n"), [ev(" two")]);
        assert_eq!(all(b"data:none\n\n"), [ev("none")]);
    }

    #[test]
    fn a_byte_order_mark_is_skipped_once() {
        assert_eq!(all("\u{feff}data: x\n\n".as_bytes()), [ev("x")]);
    }

    #[test]
    fn chunks_may_split_anywhere_including_inside_a_character() {
        let input = "id: 3\ndata: h\u{e9}llo \u{1f600}\n\n".as_bytes();
        let want = all(input);
        assert_eq!(want.len(), 1);
        for cut in 0..input.len() {
            let mut p = SseParser::new();
            let mut got = p.feed(&input[..cut]);
            got.extend(p.feed(&input[cut..]));
            assert_eq!(got, want, "cut at {cut}");
        }
        // And one byte at a time.
        let mut p = SseParser::new();
        let got: Vec<SseEvent> = input.iter().flat_map(|b| p.feed(&[*b])).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn id_and_retry_persist_for_resumption() {
        let mut p = SseParser::new();
        p.feed(b"id: 1\nretry: 2500\ndata: a\n\n");
        let second = p.feed(b"data: b\n\n");
        assert_eq!(second[0].id.as_deref(), Some("1"), "id persists");
        assert_eq!(p.last_event_id(), Some("1"));
        assert_eq!(p.retry_ms(), Some(2500));
        p.feed(b"id: 9\ndata: c\n\n");
        assert_eq!(p.last_event_id(), Some("9"));
        p.feed(b"retry: soon\ndata: d\n\n");
        assert_eq!(p.retry_ms(), Some(2500), "a non-numeric retry is ignored");
        p.feed(b"id: a\0b\ndata: e\n\n");
        assert_eq!(p.last_event_id(), Some("9"), "an id with NUL is ignored");
    }

    #[test]
    fn several_events_in_one_chunk() {
        assert_eq!(all(b"data: 1\n\ndata: 2\n\n"), [ev("1"), ev("2")]);
    }

    #[test]
    fn an_unfinished_event_waits_for_its_blank_line() {
        let mut p = SseParser::new();
        assert_eq!(p.feed(b"data: partial\n"), []);
        assert_eq!(p.feed(b"\n"), [ev("partial")]);
    }
}
